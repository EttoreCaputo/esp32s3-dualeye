#include "link.h"

#include <stdarg.h>
#include <stdio.h>
#include <string.h>

#include "cJSON.h"
#include "driver/usb_serial_jtag.h"
#include "driver/usb_serial_jtag_vfs.h"
#include "esp_app_desc.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_random.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "freertos/task.h"
#include "link_crypto.h"
#include "net.h"

/* TX holds one largest frame with room to spare. The driver drops input it
 * has no room for (USB gets no back-pressure), so RX takes a burst of about
 * 100 ms at full speed while the link task catches up. */
#define LINK_TX_RING 5120
#define LINK_RX_RING 8192
#define LINK_FRAME_MAX LINK_ENCODED_MAX(LINK_MAX_WIRE_PAYLOAD)
#define LINK_LOG_LINE_MAX 1024
#define LINK_TASK_STACK 8192
/* Above LVGL (5): draining the RX buffer must not wait for a redraw. */
#define LINK_TASK_PRIORITY 6
/* How long a frame may wait for the host to make room. Logs wait less, and
 * not at all once the host has stopped reading, so a closed port never
 * slows down the task that logs. */
#define LINK_TX_WAIT_MS 200
#define LINK_LOG_WAIT_MS 20
/* A host on WiFi has this long to prove it's paired. */
#define LINK_AUTH_TIMEOUT_US (5 * 1000 * 1000)
#define RPC_UNAUTHORIZED -32001

static const char *TAG = "link";

/* What one transport's bytes have built of the next frame so far. */
typedef struct {
    uint8_t *buf;
    size_t fill;
    bool overflow;
} rx_state_t;

static SemaphoreHandle_t s_tx_lock;
static SemaphoreHandle_t s_log_lock;
static uint8_t *s_tx_buf;
static uint8_t *s_seal_buf;
static uint8_t *s_plain;
static rx_state_t s_rx_usb;
static rx_state_t s_rx_wifi;
static char *s_log_buf;
static size_t s_log_len;
static bool s_congested;
static link_stats_t s_stats;
static link_rx_cb_t s_rx_cb[LINK_CHAN_AUDIO_DOWN + 1];

/* Where answers go; changed by the link task with s_tx_lock held. */
static volatile link_transport_t s_active = LINK_USB;
static link_transport_t s_rx_from = LINK_USB;

/* The host connected on WiFi. The link task changes it with s_tx_lock held;
 * senders read it with s_tx_lock held. */
static struct {
    uint16_t gen;
    bool open;
    bool authed;
    uint8_t nb[LINK_NONCE_BYTES];
    int64_t since_us;
    link_cipher_t cipher;
} s_wifi;

static bool holds(SemaphoreHandle_t lock)
{
    return xSemaphoreGetMutexHolder(lock) == xTaskGetCurrentTaskHandle();
}

static esp_err_t send_frame(link_chan_t chan, const void *payload, size_t len, TickType_t lock_wait,
                            TickType_t write_wait)
{
    if (s_tx_lock == NULL || len > LINK_MAX_PAYLOAD) {
        return ESP_ERR_INVALID_ARG;
    }
    if (holds(s_tx_lock)) {
        return ESP_ERR_INVALID_STATE;
    }
    if (xSemaphoreTake(s_tx_lock, lock_wait) != pdTRUE) {
        s_stats.tx_dropped++;
        return ESP_ERR_TIMEOUT;
    }
    esp_err_t err = ESP_ERR_INVALID_STATE;
    if (s_active == LINK_USB) {
        if (usb_serial_jtag_is_connected()) {
            size_t n = link_frame_encode((uint8_t) chan, payload, len, s_tx_buf);
            err = usb_serial_jtag_write_bytes(s_tx_buf, n, write_wait) == (int) n ? ESP_OK : ESP_ERR_TIMEOUT;
        }
    } else if (s_wifi.authed) {
        if (link_seal(&s_wifi.cipher, (uint8_t) chan, payload, len, s_seal_buf)) {
            size_t n = link_frame_encode((uint8_t) chan, s_seal_buf, len + LINK_SEAL_OVERHEAD, s_tx_buf);
            // Never without a wait: a frame half sent breaks the connection.
            uint32_t wait_ms = write_wait * portTICK_PERIOD_MS;
            err = net_send(s_wifi.gen, s_tx_buf, n, wait_ms < LINK_LOG_WAIT_MS ? LINK_LOG_WAIT_MS : wait_ms);
            if (err == ESP_OK) {
                link_cipher_commit(&s_wifi.cipher);
            }
        }
    }
    s_congested = err != ESP_OK;
    if (err == ESP_OK) {
        s_stats.tx_frames++;
    } else {
        s_stats.tx_dropped++;
    }
    xSemaphoreGive(s_tx_lock);
    return err;
}

esp_err_t link_send_timeout(link_chan_t chan, const void *payload, size_t len, uint32_t wait_ms)
{
    TickType_t wait = pdMS_TO_TICKS(wait_ms);
    return send_frame(chan, payload, len, wait, wait);
}

esp_err_t link_send(link_chan_t chan, const void *payload, size_t len)
{
    if (chan != LINK_CHAN_LOG) {
        return link_send_timeout(chan, payload, len, LINK_TX_WAIT_MS);
    }
    TickType_t wait = pdMS_TO_TICKS(LINK_LOG_WAIT_MS);
    return send_frame(chan, payload, len, wait, s_congested ? 0 : wait);
}

/** Send every complete line in the log buffer; with `all`, the rest too. */
static void flush_log_lines(bool all)
{
    for (;;) {
        char *nl = memchr(s_log_buf, '\n', s_log_len);
        if (nl == NULL && !(all && s_log_len > 0)) {
            return;
        }
        size_t line = nl != NULL ? (size_t) (nl - s_log_buf) : s_log_len;
        size_t used = nl != NULL ? line + 1 : line;
        while (line > 0 && s_log_buf[line - 1] == '\r') {
            line--;
        }
        if (line > 0) {
            link_send(LINK_CHAN_LOG, s_log_buf, line);
        }
        memmove(s_log_buf, s_log_buf + used, s_log_len - used);
        s_log_len -= used;
    }
}

static int log_vprintf(const char *fmt, va_list args)
{
    // A log from inside link_send (the driver complaining) would deadlock.
    if (holds(s_log_lock) || holds(s_tx_lock)) {
        return 0;
    }
    if (xSemaphoreTake(s_log_lock, pdMS_TO_TICKS(LINK_LOG_WAIT_MS)) != pdTRUE) {
        s_stats.tx_dropped++;
        return 0;
    }
    size_t room = LINK_LOG_LINE_MAX - s_log_len;
    int n = vsnprintf(s_log_buf + s_log_len, room, fmt, args);
    if (n > 0) {
        s_log_len += (size_t) n < room ? (size_t) n : room - 1;
    }
    flush_log_lines(s_log_len >= LINK_LOG_LINE_MAX - 1);
    xSemaphoreGive(s_log_lock);
    return n;
}

void link_log_printf(const char *fmt, ...)
{
    char line[160];
    va_list args;
    va_start(args, fmt);
    int n = vsnprintf(line, sizeof(line), fmt, args);
    va_end(args);
    if (n > 0) {
        link_send(LINK_CHAN_LOG, line, (size_t) n < sizeof(line) ? (size_t) n : sizeof(line) - 1);
    }
}

void link_on_receive(link_chan_t chan, link_rx_cb_t cb)
{
    if ((size_t) chan < sizeof(s_rx_cb) / sizeof(s_rx_cb[0])) {
        s_rx_cb[chan] = cb;
    }
}

void link_get_stats(link_stats_t *out)
{
    *out = s_stats;
}

link_transport_t link_active(void)
{
    return s_active;
}

link_transport_t link_rx_transport(void)
{
    return s_rx_from;
}

const char *link_transport_name(link_transport_t t)
{
    return t == LINK_WIFI ? "wifi" : "usb";
}

/** A frame outside the session's encryption, to the host on WiFi: the
 * challenge and the answer to `auth`. With s_tx_lock held. */
static void wifi_send_plain(const char *json)
{
    size_t len = strlen(json);
    if (len > LINK_MAX_PAYLOAD) {
        return;
    }
    size_t n = link_frame_encode(LINK_CHAN_CTRL, (const uint8_t *) json, len, s_tx_buf);
    net_send(s_wifi.gen, s_tx_buf, n, LINK_TX_WAIT_MS);
}

static void set_active(link_transport_t t)
{
    if (s_active == t) {
        return;
    }
    xSemaphoreTake(s_tx_lock, portMAX_DELAY);
    s_active = t;
    uint16_t gen = s_wifi.gen;
    bool drop = t == LINK_USB && s_wifi.open;
    xSemaphoreGive(s_tx_lock);
    if (drop) {
        // A host on USB took over: the one on WiFi learns it's over.
        net_drop(gen);
    }
    ESP_LOGI(TAG, "host now on %s", link_transport_name(t));
}

/** A WiFi host's first request: `auth`, with its nonce and its proof that
 * it holds the pairing key. */
static void wifi_auth(const uint8_t *payload, size_t len)
{
    cJSON *req = cJSON_ParseWithLength((const char *) payload, len);
    const cJSON *method = cJSON_GetObjectItemCaseSensitive(req, "method");
    const cJSON *id = cJSON_GetObjectItemCaseSensitive(req, "id");
    const cJSON *params = cJSON_GetObjectItemCaseSensitive(req, "params");
    const cJSON *nonce = cJSON_GetObjectItemCaseSensitive(params, "nonce");
    const cJSON *proof = cJSON_GetObjectItemCaseSensitive(params, "proof");
    uint8_t key[LINK_KEY_BYTES];
    uint8_t nh[LINK_NONCE_BYTES];
    uint8_t got[LINK_PROOF_BYTES];
    uint8_t want[LINK_PROOF_BYTES];
    bool ok = cJSON_IsString(method) && strcmp(method->valuestring, "auth") == 0 && cJSON_IsString(nonce)
              && cJSON_IsString(proof) && link_unhex(nonce->valuestring, nh, sizeof(nh))
              && link_unhex(proof->valuestring, got, sizeof(got)) && net_key(key)
              && link_crypto_derive(key, "dualeye host", s_wifi.nb, nh, want);
    uint8_t diff = ok ? 0 : 1;
    for (size_t i = 0; ok && i < sizeof(want); i++) {
        diff |= got[i] ^ want[i];
    }
    ok = diff == 0;

    cJSON *reply = cJSON_CreateObject();
    cJSON_AddStringToObject(reply, "jsonrpc", "2.0");
    cJSON_AddItemToObject(reply, "id", id != NULL ? cJSON_Duplicate(id, true) : cJSON_CreateNull());
    uint8_t mine[LINK_PROOF_BYTES];
    if (ok && link_crypto_derive(key, "dualeye board", s_wifi.nb, nh, mine)) {
        char hex[2 * LINK_PROOF_BYTES + 1];
        link_hex(mine, sizeof(mine), hex);
        cJSON *result = cJSON_AddObjectToObject(reply, "result");
        cJSON_AddStringToObject(result, "proof", hex);
    } else {
        ok = false;
        cJSON *err = cJSON_AddObjectToObject(reply, "error");
        cJSON_AddNumberToObject(err, "code", RPC_UNAUTHORIZED);
        cJSON_AddStringToObject(err, "message", "not paired with this board: set its WiFi up over USB");
    }
    char *text = cJSON_PrintUnformatted(reply);
    cJSON_Delete(reply);
    cJSON_Delete(req);

    xSemaphoreTake(s_tx_lock, portMAX_DELAY);
    if (text != NULL) {
        wifi_send_plain(text);
    }
    // Sealed from the next frame on, both ways.
    if (ok) {
        s_wifi.authed = link_cipher_start(&s_wifi.cipher, key, s_wifi.nb, nh);
        ok = s_wifi.authed;
    }
    uint16_t gen = s_wifi.gen;
    xSemaphoreGive(s_tx_lock);
    cJSON_free(text);
    memset(key, 0, sizeof(key));
    if (ok) {
        ESP_LOGI(TAG, "host on WiFi paired");
    } else {
        ESP_LOGW(TAG, "host on WiFi isn't paired, closing");
        net_drop(gen);
    }
}

static bool is_hello(const uint8_t *payload, size_t len)
{
    cJSON *msg = cJSON_ParseWithLength((const char *) payload, len);
    const cJSON *method = cJSON_GetObjectItemCaseSensitive(msg, "method");
    bool hello = cJSON_IsString(method) && strcmp(method->valuestring, "hello") == 0;
    cJSON_Delete(msg);
    return hello;
}

static void handle_piece(link_transport_t from, uint8_t *buf, size_t n)
{
    uint8_t chan = 0;
    uint8_t *payload = NULL;
    size_t len = 0;
    if (!link_frame_decode(buf, n, &chan, &payload, &len)) {
        s_stats.rx_bad++;
        return;
    }
    if (from == LINK_WIFI) {
        if (!s_wifi.authed) {
            if (chan == LINK_CHAN_CTRL) {
                wifi_auth(payload, len);
            }
            return;
        }
        size_t plain = 0;
        if (!link_open(&s_wifi.cipher, chan, payload, len, s_plain, &plain)) {
            // Tampered with, or out of step: the connection can't go on.
            s_stats.rx_bad++;
            net_drop(s_wifi.gen);
            return;
        }
        payload = s_plain;
        len = plain;
    } else if (len > LINK_MAX_PAYLOAD) {
        s_stats.rx_bad++;
        return;
    }
    s_stats.rx_frames++;
    // There's room for a terminator: the CRC's on USB, the tag's on WiFi.
    payload[len] = '\0';
    if (from != s_active) {
        // The other host's frames are dropped, unless it says hello.
        if (chan != LINK_CHAN_CTRL || !is_hello(payload, len)) {
            s_stats.rx_ignored++;
            return;
        }
        set_active(from);
    }
    s_rx_from = from;
    if (chan < sizeof(s_rx_cb) / sizeof(s_rx_cb[0]) && s_rx_cb[chan] != NULL) {
        s_rx_cb[chan](payload, len);
    }
}

static void feed(rx_state_t *st, link_transport_t from, const uint8_t *data, size_t n)
{
    for (size_t i = 0; i < n; i++) {
        uint8_t b = data[i];
        if (b == 0x00) {
            if (st->overflow) {
                s_stats.rx_bad++;
            } else if (st->fill > 0) {
                handle_piece(from, st->buf, st->fill);
            }
            st->fill = 0;
            st->overflow = false;
        } else if (st->fill < LINK_FRAME_MAX) {
            st->buf[st->fill++] = b;
        } else {
            st->overflow = true;
        }
    }
}

static void wifi_event(const net_chunk_t *c)
{
    if (c->len == NET_CONNECTED) {
        xSemaphoreTake(s_tx_lock, portMAX_DELAY);
        link_cipher_end(&s_wifi.cipher);
        s_wifi.gen = c->gen;
        s_wifi.open = true;
        s_wifi.authed = false;
        s_wifi.since_us = esp_timer_get_time();
        esp_fill_random(s_wifi.nb, sizeof(s_wifi.nb));
        s_rx_wifi.fill = 0;
        s_rx_wifi.overflow = false;
        char nonce[2 * LINK_NONCE_BYTES + 1];
        link_hex(s_wifi.nb, sizeof(s_wifi.nb), nonce);
        char challenge[256];
        snprintf(challenge, sizeof(challenge),
                 "{\"jsonrpc\":\"2.0\",\"method\":\"auth/challenge\",\"params\":{\"nonce\":\"%s\",\"id\":\"%s\","
                 "\"protocol\":2,\"firmware\":\"%s\"}}",
                 nonce, net_board_id(), esp_app_get_description()->version);
        wifi_send_plain(challenge);
        xSemaphoreGive(s_tx_lock);
    } else if (c->len == NET_CLOSED) {
        if (c->gen != s_wifi.gen || !s_wifi.open) {
            return;
        }
        xSemaphoreTake(s_tx_lock, portMAX_DELAY);
        link_cipher_end(&s_wifi.cipher);
        s_wifi.open = false;
        s_wifi.authed = false;
        bool was_active = s_active == LINK_WIFI;
        s_active = LINK_USB;
        xSemaphoreGive(s_tx_lock);
        if (was_active) {
            ESP_LOGI(TAG, "host on WiFi gone, back to USB");
        }
    } else if (c->len > 0 && c->gen == s_wifi.gen && s_wifi.open) {
        feed(&s_rx_wifi, LINK_WIFI, c->data, (size_t) c->len);
    }
}

static void link_rx_task(void *arg)
{
    uint8_t chunk[256];
    net_chunk_t *net = heap_caps_malloc(sizeof(net_chunk_t), MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    ESP_LOGI(TAG, "protocol v2 up");
    while (true) {
        QueueHandle_t queue = net != NULL ? net_rx_queue() : NULL;
        if (queue == NULL) {
            // USB only, as long as the WiFi has never been on: it is turned
            // on over USB, so this wakes up for it.
            int n = usb_serial_jtag_read_bytes(chunk, sizeof(chunk), portMAX_DELAY);
            feed(&s_rx_usb, LINK_USB, chunk, n > 0 ? (size_t) n : 0);
            continue;
        }
        // Wait on the host's transport, look at the other one in passing.
        bool wifi = s_active == LINK_WIFI;
        int n = usb_serial_jtag_read_bytes(chunk, sizeof(chunk), wifi ? 0 : 1);
        feed(&s_rx_usb, LINK_USB, chunk, n > 0 ? (size_t) n : 0);
        TickType_t wait = wifi ? 1 : 0;
        while (xQueueReceive(queue, net, wait) == pdTRUE) {
            wifi_event(net);
            wait = 0;
        }
        if (s_wifi.open && !s_wifi.authed && esp_timer_get_time() - s_wifi.since_us > LINK_AUTH_TIMEOUT_US) {
            s_wifi.since_us = esp_timer_get_time();
            net_drop(s_wifi.gen);
        }
    }
}

esp_err_t link_init(void)
{
    s_tx_lock = xSemaphoreCreateMutex();
    s_log_lock = xSemaphoreCreateMutex();
    s_tx_buf = heap_caps_malloc(LINK_FRAME_MAX, MALLOC_CAP_SPIRAM);
    s_seal_buf = heap_caps_malloc(LINK_MAX_WIRE_PAYLOAD, MALLOC_CAP_SPIRAM);
    s_plain = heap_caps_malloc(LINK_MAX_PAYLOAD + 1, MALLOC_CAP_SPIRAM);
    s_rx_usb.buf = heap_caps_malloc(LINK_FRAME_MAX, MALLOC_CAP_SPIRAM);
    s_rx_wifi.buf = heap_caps_malloc(LINK_FRAME_MAX, MALLOC_CAP_SPIRAM);
    s_log_buf = heap_caps_malloc(LINK_LOG_LINE_MAX, MALLOC_CAP_SPIRAM);
    if (!s_tx_lock || !s_log_lock || !s_tx_buf || !s_seal_buf || !s_plain || !s_rx_usb.buf || !s_rx_wifi.buf
        || !s_log_buf) {
        return ESP_ERR_NO_MEM;
    }
    usb_serial_jtag_driver_config_t cfg = {
        .tx_buffer_size = LINK_TX_RING,
        .rx_buffer_size = LINK_RX_RING,
    };
    esp_err_t err = usb_serial_jtag_driver_install(&cfg);
    if (err != ESP_OK) {
        return err;
    }
    // A stray printf now goes through the driver too: raw text between
    // frames rather than bytes in the middle of one.
    usb_serial_jtag_vfs_use_driver();
    esp_log_set_vprintf(log_vprintf);
    return ESP_OK;
}

void link_start(void)
{
    BaseType_t ok = xTaskCreate(link_rx_task, "link", LINK_TASK_STACK, NULL, LINK_TASK_PRIORITY, NULL);
    if (ok != pdPASS) {
        ESP_LOGE(TAG, "failed to start the link task");
    }
}
