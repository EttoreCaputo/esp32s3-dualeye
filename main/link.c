#include "link.h"

#include <stdarg.h>
#include <stdio.h>
#include <string.h>

#include "driver/usb_serial_jtag.h"
#include "driver/usb_serial_jtag_vfs.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "freertos/task.h"

/* TX holds one largest frame with room to spare. The driver drops input it
 * has no room for (USB gets no back-pressure), so RX takes a burst of about
 * 100 ms at full speed while the link task catches up. */
#define LINK_TX_RING 5120
#define LINK_RX_RING 8192
#define LINK_FRAME_MAX LINK_ENCODED_MAX(LINK_MAX_PAYLOAD)
#define LINK_LOG_LINE_MAX 1024
#define LINK_TASK_STACK 8192
/* Above LVGL (5): draining the RX buffer must not wait for a redraw. */
#define LINK_TASK_PRIORITY 6
/* How long a frame may wait for the host to make room. Logs wait less, and
 * not at all once the host has stopped reading, so a closed port never
 * slows down the task that logs. */
#define LINK_TX_WAIT_MS 200
#define LINK_LOG_WAIT_MS 20

static const char *TAG = "link";

static SemaphoreHandle_t s_tx_lock;
static SemaphoreHandle_t s_log_lock;
static uint8_t *s_tx_buf;
static uint8_t *s_rx_buf;
static char *s_log_buf;
static size_t s_log_len;
static bool s_congested;
static link_stats_t s_stats;
static link_rx_cb_t s_rx_cb[LINK_CHAN_AUDIO_DOWN + 1];

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
    if (usb_serial_jtag_is_connected()) {
        size_t n = link_frame_encode((uint8_t) chan, payload, len, s_tx_buf);
        err = usb_serial_jtag_write_bytes(s_tx_buf, n, write_wait) == (int) n ? ESP_OK : ESP_ERR_TIMEOUT;
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

static void handle_piece(size_t n)
{
    uint8_t chan = 0;
    uint8_t *payload = NULL;
    size_t len = 0;
    if (!link_frame_decode(s_rx_buf, n, &chan, &payload, &len)) {
        s_stats.rx_bad++;
        return;
    }
    s_stats.rx_frames++;
    // The CRC after the payload is checked already: room for a terminator.
    payload[len] = '\0';
    if (chan < sizeof(s_rx_cb) / sizeof(s_rx_cb[0]) && s_rx_cb[chan] != NULL) {
        s_rx_cb[chan](payload, len);
    }
}

static void link_rx_task(void *arg)
{
    uint8_t chunk[256];
    size_t fill = 0;
    bool overflow = false;
    ESP_LOGI(TAG, "protocol v2 up");
    while (true) {
        int n = usb_serial_jtag_read_bytes(chunk, sizeof(chunk), portMAX_DELAY);
        for (int i = 0; i < n; i++) {
            uint8_t b = chunk[i];
            if (b == 0x00) {
                if (overflow) {
                    s_stats.rx_bad++;
                } else if (fill > 0) {
                    handle_piece(fill);
                }
                fill = 0;
                overflow = false;
            } else if (fill < LINK_FRAME_MAX) {
                s_rx_buf[fill++] = b;
            } else {
                overflow = true;
            }
        }
    }
}

esp_err_t link_init(void)
{
    s_tx_lock = xSemaphoreCreateMutex();
    s_log_lock = xSemaphoreCreateMutex();
    s_tx_buf = heap_caps_malloc(LINK_FRAME_MAX, MALLOC_CAP_SPIRAM);
    s_rx_buf = heap_caps_malloc(LINK_FRAME_MAX, MALLOC_CAP_SPIRAM);
    s_log_buf = heap_caps_malloc(LINK_LOG_LINE_MAX, MALLOC_CAP_SPIRAM);
    if (!s_tx_lock || !s_log_lock || !s_tx_buf || !s_rx_buf || !s_log_buf) {
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
