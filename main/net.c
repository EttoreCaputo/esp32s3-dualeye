#include "net.h"

#include <errno.h>
#include <string.h>

#include "esp_app_desc.h"
#include "esp_event.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_netif.h"
#include "esp_random.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "freertos/idf_additions.h"
#include "freertos/semphr.h"
#include "freertos/task.h"
#include "lwip/sockets.h"
#include "mdns.h"
#include "nvs.h"

#define NVS_NAMESPACE "dualeye_net"
#define ON_KEY "on"
#define SSID_KEY "ssid"
#define PASS_KEY "pass"
#define PAIR_KEY "key"

/* Chunks waiting for the link task: 8 KB, in PSRAM. */
#define RX_QUEUE_LEN 16
/* The TCP task only moves bytes, no flash: its stack can be in PSRAM. */
#define TCP_TASK_STACK 4096
#define TCP_TASK_PRIORITY 5
#define SELECT_MS 200
#define RETRY_MIN_MS 1000
#define RETRY_MAX_MS 30000
#define SCAN_MAX 20
/* A host that went away without closing: found out in about 11 s. */
#define KEEPALIVE_IDLE_S 5
#define KEEPALIVE_INTERVAL_S 2
#define KEEPALIVE_COUNT 3

static const char *TAG = "net";

typedef enum {
    ST_OFF,
    ST_CONNECTING,
    ST_CONNECTED,
    ST_NO_NETWORK,
    ST_WRONG_PASSWORD,
    ST_DISCONNECTED,
} net_state_t;

static const char *const STATE_NAMES[] = {
    [ST_OFF] = "off",
    [ST_CONNECTING] = "connecting",
    [ST_CONNECTED] = "connected",
    [ST_NO_NETWORK] = "no_network",
    [ST_WRONG_PASSWORD] = "wrong_password",
    [ST_DISCONNECTED] = "disconnected",
};

/* Settings and state: s_lock. */
static SemaphoreHandle_t s_lock;
static bool s_enabled;
static char s_ssid[33];
static char s_pass[65];
static uint8_t s_key[LINK_KEY_BYTES];
static bool s_has_key;
static net_state_t s_state;
static esp_ip4_addr_t s_ip;
static bool s_scanning;
static uint32_t s_retry_ms = RETRY_MIN_MS;

static char s_id[13];
static char s_host[24];
static bool s_up;          /* driver, netif, mDNS and the TCP task, once */
static bool s_started;     /* esp_wifi_start() */
static esp_timer_handle_t s_retry;

/* The connection: s_sock_lock. */
static SemaphoreHandle_t s_sock_lock;
static QueueHandle_t s_rx_queue;
static int s_client = -1;
static uint16_t s_gen;
static volatile bool s_drop;
/* The TCP task's event chunk, in PSRAM. */
static net_chunk_t *s_event;

static void lock(void)
{
    xSemaphoreTake(s_lock, portMAX_DELAY);
}

static void unlock(void)
{
    xSemaphoreGive(s_lock);
}

/* ---- settings ---- */

static void load(void)
{
    nvs_handle_t nvs;
    if (nvs_open(NVS_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return;
    }
    uint8_t on = 0;
    if (nvs_get_u8(nvs, ON_KEY, &on) == ESP_OK) {
        s_enabled = on != 0;
    }
    size_t len = sizeof(s_ssid);
    if (nvs_get_str(nvs, SSID_KEY, s_ssid, &len) != ESP_OK) {
        s_ssid[0] = '\0';
    }
    len = sizeof(s_pass);
    if (nvs_get_str(nvs, PASS_KEY, s_pass, &len) != ESP_OK) {
        s_pass[0] = '\0';
    }
    len = sizeof(s_key);
    s_has_key = nvs_get_blob(nvs, PAIR_KEY, s_key, &len) == ESP_OK && len == sizeof(s_key);
    nvs_close(nvs);
}

static void save(void)
{
    nvs_handle_t nvs;
    esp_err_t err = nvs_open(NVS_NAMESPACE, NVS_READWRITE, &nvs);
    if (err != ESP_OK) {
        ESP_LOGW(TAG, "saving the WiFi settings failed: %s", esp_err_to_name(err));
        return;
    }
    nvs_set_u8(nvs, ON_KEY, s_enabled);
    nvs_set_str(nvs, SSID_KEY, s_ssid);
    nvs_set_str(nvs, PASS_KEY, s_pass);
    if (s_has_key) {
        nvs_set_blob(nvs, PAIR_KEY, s_key, sizeof(s_key));
    } else {
        nvs_erase_key(nvs, PAIR_KEY);
    }
    err = nvs_commit(nvs);
    nvs_close(nvs);
    if (err != ESP_OK) {
        ESP_LOGW(TAG, "saving the WiFi settings failed: %s", esp_err_to_name(err));
    }
}

/* ---- the connection ---- */

/** Tell the link task a host connected or went. TCP task only. */
static void push_event(uint16_t gen, int16_t what)
{
    s_event->gen = gen;
    s_event->len = what;
    xQueueSend(s_rx_queue, s_event, portMAX_DELAY);
}

static void close_client(void)
{
    xSemaphoreTake(s_sock_lock, portMAX_DELAY);
    int fd = s_client;
    uint16_t gen = s_gen;
    s_client = -1;
    s_drop = false;
    xSemaphoreGive(s_sock_lock);
    if (fd >= 0) {
        close(fd);
        push_event(gen, NET_CLOSED);
        ESP_LOGI(TAG, "host on WiFi gone");
    }
}

static void take_client(int fd)
{
    int on = 1;
    setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &on, sizeof(on));
    setsockopt(fd, SOL_SOCKET, SO_KEEPALIVE, &on, sizeof(on));
    int idle = KEEPALIVE_IDLE_S;
    int interval = KEEPALIVE_INTERVAL_S;
    int count = KEEPALIVE_COUNT;
    setsockopt(fd, IPPROTO_TCP, TCP_KEEPIDLE, &idle, sizeof(idle));
    setsockopt(fd, IPPROTO_TCP, TCP_KEEPINTVL, &interval, sizeof(interval));
    setsockopt(fd, IPPROTO_TCP, TCP_KEEPCNT, &count, sizeof(count));
    // The newest host wins: an old connection is most likely a computer
    // that went to sleep without closing it.
    close_client();
    xSemaphoreTake(s_sock_lock, portMAX_DELAY);
    s_client = fd;
    uint16_t gen = ++s_gen;
    xSemaphoreGive(s_sock_lock);
    push_event(gen, NET_CONNECTED);
}

static void tcp_task(void *arg)
{
    net_chunk_t *chunk = heap_caps_malloc(sizeof(net_chunk_t), MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    int listener = socket(AF_INET, SOCK_STREAM, IPPROTO_IP);
    struct sockaddr_in addr = {
        .sin_family = AF_INET,
        .sin_port = htons(NET_TCP_PORT),
        .sin_addr.s_addr = htonl(INADDR_ANY),
    };
    int on = 1;
    if (chunk == NULL || listener < 0 || setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &on, sizeof(on)) != 0
        || bind(listener, (struct sockaddr *) &addr, sizeof(addr)) != 0 || listen(listener, 1) != 0) {
        ESP_LOGE(TAG, "can't listen on port %d (errno %d)", NET_TCP_PORT, errno);
        vTaskDeleteWithCaps(NULL);
        return;
    }
    while (true) {
        if (s_drop) {
            close_client();
        }
        int client = s_client;
        fd_set rd;
        FD_ZERO(&rd);
        FD_SET(listener, &rd);
        int max = listener;
        if (client >= 0) {
            FD_SET(client, &rd);
            max = client > max ? client : max;
        }
        struct timeval tv = {.tv_sec = 0, .tv_usec = SELECT_MS * 1000};
        int ready = select(max + 1, &rd, NULL, NULL, &tv);
        if (ready < 0) {
            vTaskDelay(pdMS_TO_TICKS(SELECT_MS));
            continue;
        }
        if (FD_ISSET(listener, &rd)) {
            int fd = accept(listener, NULL, NULL);
            if (fd >= 0) {
                lock();
                bool enabled = s_enabled;
                unlock();
                if (enabled) {
                    take_client(fd);
                    ESP_LOGI(TAG, "host connected on WiFi");
                    continue;
                }
                close(fd);
            }
        }
        if (client >= 0 && client == s_client && FD_ISSET(client, &rd)) {
            int n = recv(client, chunk->data, sizeof(chunk->data), 0);
            if (n > 0) {
                chunk->gen = s_gen;
                chunk->len = (int16_t) n;
                // Waiting here holds the host back: TCP's own flow control.
                xQueueSend(s_rx_queue, chunk, portMAX_DELAY);
            } else {
                close_client();
            }
        }
    }
}

QueueHandle_t net_rx_queue(void)
{
    return s_rx_queue;
}

esp_err_t net_send(uint16_t gen, const uint8_t *buf, size_t len, uint32_t wait_ms)
{
    if (s_sock_lock == NULL || xSemaphoreTake(s_sock_lock, pdMS_TO_TICKS(wait_ms)) != pdTRUE) {
        return ESP_ERR_TIMEOUT;
    }
    int fd = s_client;
    if (fd < 0 || s_gen != gen || s_drop) {
        xSemaphoreGive(s_sock_lock);
        return ESP_FAIL;
    }
    int64_t deadline = esp_timer_get_time() + (int64_t) wait_ms * 1000;
    size_t done = 0;
    esp_err_t err = ESP_OK;
    while (done < len) {
        int n = send(fd, buf + done, len - done, MSG_DONTWAIT);
        if (n > 0) {
            done += (size_t) n;
            continue;
        }
        if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            int64_t left = deadline - esp_timer_get_time();
            if (left > 0) {
                fd_set wr;
                FD_ZERO(&wr);
                FD_SET(fd, &wr);
                struct timeval tv = {.tv_sec = (long) (left / 1000000), .tv_usec = (long) (left % 1000000)};
                select(fd + 1, NULL, &wr, NULL, &tv);
                continue;
            }
            // Nothing of it went: the frame is dropped, the stream is still whole.
            err = done == 0 ? ESP_ERR_TIMEOUT : ESP_FAIL;
        } else {
            err = ESP_FAIL;
        }
        break;
    }
    if (err == ESP_FAIL) {
        shutdown(fd, SHUT_RDWR);
        s_drop = true;
    }
    xSemaphoreGive(s_sock_lock);
    return err;
}

void net_drop(uint16_t gen)
{
    if (s_sock_lock == NULL) {
        return;
    }
    xSemaphoreTake(s_sock_lock, portMAX_DELAY);
    if (s_client >= 0 && s_gen == gen) {
        shutdown(s_client, SHUT_RDWR);
        s_drop = true;
    }
    xSemaphoreGive(s_sock_lock);
}

/* ---- WiFi ---- */

static void schedule_retry(void)
{
    esp_timer_stop(s_retry);
    esp_timer_start_once(s_retry, (uint64_t) s_retry_ms * 1000);
    s_retry_ms = s_retry_ms * 2 > RETRY_MAX_MS ? RETRY_MAX_MS : s_retry_ms * 2;
}

static void retry_cb(void *arg)
{
    lock();
    bool go = s_enabled && s_ssid[0] != '\0' && !s_scanning && s_state != ST_CONNECTED;
    unlock();
    if (go) {
        esp_wifi_connect();
    }
}

static void on_event(void *arg, esp_event_base_t base, int32_t id, void *data)
{
    if (base == WIFI_EVENT && id == WIFI_EVENT_STA_START) {
        lock();
        bool go = s_enabled && s_ssid[0] != '\0' && !s_scanning;
        unlock();
        if (go) {
            esp_wifi_connect();
        }
    } else if (base == WIFI_EVENT && id == WIFI_EVENT_STA_DISCONNECTED) {
        const wifi_event_sta_disconnected_t *ev = data;
        lock();
        s_ip.addr = 0;
        bool retry = s_enabled && !s_scanning;
        if (retry) {
            switch (ev->reason) {
            case WIFI_REASON_NO_AP_FOUND:
            case WIFI_REASON_NO_AP_FOUND_W_COMPATIBLE_SECURITY:
            case WIFI_REASON_NO_AP_FOUND_IN_AUTHMODE_THRESHOLD:
            case WIFI_REASON_NO_AP_FOUND_IN_RSSI_THRESHOLD:
                s_state = ST_NO_NETWORK;
                break;
            case WIFI_REASON_AUTH_FAIL:
            case WIFI_REASON_4WAY_HANDSHAKE_TIMEOUT:
            case WIFI_REASON_HANDSHAKE_TIMEOUT:
                s_state = ST_WRONG_PASSWORD;
                break;
            default:
                s_state = ST_DISCONNECTED;
                break;
            }
            schedule_retry();
        }
        unlock();
        if (retry) {
            ESP_LOGW(TAG, "WiFi: not connected (reason %d), retrying", ev->reason);
        }
    } else if (base == IP_EVENT && id == IP_EVENT_STA_GOT_IP) {
        const ip_event_got_ip_t *ev = data;
        lock();
        s_state = ST_CONNECTED;
        s_ip = ev->ip_info.ip;
        s_retry_ms = RETRY_MIN_MS;
        unlock();
        ESP_LOGI(TAG, "WiFi: connected to \"%s\" as " IPSTR ", %s.local:%d", s_ssid, IP2STR(&ev->ip_info.ip), s_host,
                 NET_TCP_PORT);
    }
}

static void start_mdns(void)
{
    esp_err_t err = mdns_init();
    if (err != ESP_OK) {
        ESP_LOGW(TAG, "mDNS unavailable: %s", esp_err_to_name(err));
        return;
    }
    mdns_hostname_set(s_host);
    mdns_instance_name_set("DualEye");
    mdns_txt_item_t txt[] = {
        {"id", s_id},
        {"fw", esp_app_get_description()->version},
        {"proto", "2"},
    };
    mdns_service_add(NULL, "_dualeye", "_tcp", NET_TCP_PORT, txt, sizeof(txt) / sizeof(txt[0]));
}

/** Driver, netif, mDNS and the TCP task, the first time the WiFi is wanted. */
static bool bring_up(void)
{
    if (s_up) {
        return true;
    }
    esp_err_t err = esp_netif_init();
    if (err == ESP_OK) {
        err = esp_event_loop_create_default();
        err = err == ESP_ERR_INVALID_STATE ? ESP_OK : err;
    }
    esp_netif_t *netif = err == ESP_OK ? esp_netif_create_default_wifi_sta() : NULL;
    if (netif == NULL) {
        ESP_LOGE(TAG, "WiFi: netif failed: %s", esp_err_to_name(err));
        return false;
    }
    wifi_init_config_t cfg = WIFI_INIT_CONFIG_DEFAULT();
    // The network is ours to keep (NVS, above), not the driver's.
    cfg.nvs_enable = false;
    err = esp_wifi_init(&cfg);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "WiFi: init failed: %s", esp_err_to_name(err));
        return false;
    }
    esp_netif_set_hostname(netif, s_host);
    esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID, on_event, NULL);
    esp_event_handler_register(IP_EVENT, IP_EVENT_STA_GOT_IP, on_event, NULL);
    esp_wifi_set_mode(WIFI_MODE_STA);
    const esp_timer_create_args_t retry = {.callback = retry_cb, .name = "wifi_retry"};
    esp_timer_create(&retry, &s_retry);

    s_sock_lock = xSemaphoreCreateMutex();
    s_rx_queue = xQueueCreateWithCaps(RX_QUEUE_LEN, sizeof(net_chunk_t), MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    s_event = heap_caps_malloc(sizeof(net_chunk_t), MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    if (s_sock_lock == NULL || s_rx_queue == NULL || s_event == NULL
        || xTaskCreatePinnedToCoreWithCaps(tcp_task, "net_tcp", TCP_TASK_STACK, NULL, TCP_TASK_PRIORITY, NULL, 0,
                                           MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT)
               != pdPASS) {
        ESP_LOGE(TAG, "WiFi: out of memory for the TCP task");
        return false;
    }
    start_mdns();
    s_up = true;
    return true;
}

/** Start, stop or reconnect to match the settings. Not with s_lock held. */
static void apply(void)
{
    lock();
    bool want = s_enabled && s_ssid[0] != '\0';
    wifi_config_t cfg = {0};
    memcpy(cfg.sta.ssid, s_ssid, strlen(s_ssid));
    memcpy(cfg.sta.password, s_pass, strlen(s_pass));
    unlock();

    if (!want) {
        if (s_started) {
            esp_timer_stop(s_retry);
            esp_wifi_stop();
            s_started = false;
        }
        if (s_sock_lock != NULL) {
            xSemaphoreTake(s_sock_lock, portMAX_DELAY);
            if (s_client >= 0) {
                shutdown(s_client, SHUT_RDWR);
                s_drop = true;
            }
            xSemaphoreGive(s_sock_lock);
        }
        lock();
        s_state = ST_OFF;
        s_ip.addr = 0;
        unlock();
        return;
    }
    if (!bring_up()) {
        return;
    }
    cfg.sta.pmf_cfg.capable = true;
    cfg.sta.scan_method = WIFI_ALL_CHANNEL_SCAN;
    cfg.sta.sort_method = WIFI_CONNECT_AP_BY_SIGNAL;
    lock();
    s_state = ST_CONNECTING;
    s_ip.addr = 0;
    s_retry_ms = RETRY_MIN_MS;
    unlock();
    esp_timer_stop(s_retry);
    if (s_started) {
        esp_wifi_disconnect();
        esp_wifi_set_config(WIFI_IF_STA, &cfg);
        esp_wifi_connect();
    } else {
        esp_wifi_set_config(WIFI_IF_STA, &cfg);
        if (esp_wifi_start() == ESP_OK) {
            s_started = true;
            // Latency over power: the board is on USB power anyway.
            esp_wifi_set_ps(WIFI_PS_NONE);
        }
    }
}

void net_init(void)
{
    s_lock = xSemaphoreCreateMutex();
    uint8_t mac[6] = {0};
    esp_read_mac(mac, ESP_MAC_WIFI_STA);
    link_hex(mac, sizeof(mac), s_id);
    snprintf(s_host, sizeof(s_host), "dualeye-%s", s_id + 6);
    load();
    if (s_enabled && s_ssid[0] != '\0') {
        ESP_LOGI(TAG, "WiFi: joining \"%s\"", s_ssid);
        apply();
    }
}

bool net_key(uint8_t key[LINK_KEY_BYTES])
{
    if (s_lock == NULL) {
        return false;
    }
    lock();
    bool has = s_has_key;
    if (has) {
        memcpy(key, s_key, LINK_KEY_BYTES);
    }
    unlock();
    return has;
}

const char *net_board_id(void)
{
    return s_id;
}

cJSON *net_status(void)
{
    cJSON *st = cJSON_CreateObject();
    lock();
    cJSON_AddBoolToObject(st, "enabled", s_enabled);
    if (s_ssid[0] != '\0') {
        cJSON_AddStringToObject(st, "ssid", s_ssid);
    } else {
        cJSON_AddNullToObject(st, "ssid");
    }
    cJSON_AddStringToObject(st, "state", STATE_NAMES[s_state]);
    if (s_state == ST_CONNECTED) {
        char ip[16];
        snprintf(ip, sizeof(ip), IPSTR, IP2STR(&s_ip));
        cJSON_AddStringToObject(st, "ip", ip);
    } else {
        cJSON_AddNullToObject(st, "ip");
    }
    bool connected = s_state == ST_CONNECTED;
    cJSON_AddBoolToObject(st, "paired", s_has_key);
    unlock();
    wifi_ap_record_t ap;
    if (connected && esp_wifi_sta_get_ap_info(&ap) == ESP_OK) {
        cJSON_AddNumberToObject(st, "rssi", ap.rssi);
    }
    char host[32];
    snprintf(host, sizeof(host), "%s.local", s_host);
    cJSON_AddStringToObject(st, "host", host);
    cJSON_AddNumberToObject(st, "port", NET_TCP_PORT);
    cJSON_AddStringToObject(st, "id", s_id);
    return st;
}

static cJSON *scan(const char **message)
{
    if (!bring_up()) {
        *message = "WiFi unavailable";
        return NULL;
    }
    lock();
    s_scanning = true;
    bool connected = s_state == ST_CONNECTED;
    unlock();
    bool started_here = !s_started;
    if (started_here) {
        if (esp_wifi_start() != ESP_OK) {
            lock();
            s_scanning = false;
            unlock();
            *message = "WiFi unavailable";
            return NULL;
        }
        s_started = true;
    } else if (!connected) {
        // Trying to join gets in the way of scanning.
        esp_timer_stop(s_retry);
        esp_wifi_disconnect();
    }
    wifi_scan_config_t sc = {.show_hidden = false};
    esp_err_t err = esp_wifi_scan_start(&sc, true);
    uint16_t n = SCAN_MAX;
    wifi_ap_record_t *recs = heap_caps_calloc(SCAN_MAX, sizeof(wifi_ap_record_t), MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    if (err == ESP_OK && recs != NULL) {
        err = esp_wifi_scan_get_ap_records(&n, recs);
    } else {
        esp_wifi_clear_ap_list();
        n = 0;
    }
    lock();
    s_scanning = false;
    bool rejoin = s_enabled && s_ssid[0] != '\0';
    unlock();
    if (started_here && !rejoin) {
        esp_wifi_stop();
        s_started = false;
    } else if (!connected && rejoin) {
        esp_wifi_connect();
    }
    if (err != ESP_OK || recs == NULL) {
        free(recs);
        *message = "the scan failed, try again";
        return NULL;
    }
    // Strongest first (the driver sorts them), each network once.
    cJSON *result = cJSON_CreateObject();
    cJSON *list = cJSON_AddArrayToObject(result, "networks");
    for (int i = 0; i < n; i++) {
        const char *ssid = (const char *) recs[i].ssid;
        bool seen = ssid[0] == '\0';
        for (int j = 0; j < i && !seen; j++) {
            seen = strcmp(ssid, (const char *) recs[j].ssid) == 0;
        }
        if (seen) {
            continue;
        }
        cJSON *net = cJSON_CreateObject();
        cJSON_AddStringToObject(net, "ssid", ssid);
        cJSON_AddNumberToObject(net, "rssi", recs[i].rssi);
        cJSON_AddBoolToObject(net, "secure", recs[i].authmode != WIFI_AUTH_OPEN);
        cJSON_AddItemToArray(list, net);
    }
    free(recs);
    return result;
}

static void add_key(cJSON *obj)
{
    char hex[2 * LINK_KEY_BYTES + 1];
    lock();
    link_hex(s_key, sizeof(s_key), hex);
    unlock();
    cJSON_AddStringToObject(obj, "key", hex);
}

static cJSON *set(const cJSON *params, const char **message)
{
    const cJSON *ssid = cJSON_GetObjectItemCaseSensitive(params, "ssid");
    const cJSON *pass = cJSON_GetObjectItemCaseSensitive(params, "password");
    const cJSON *enabled = cJSON_GetObjectItemCaseSensitive(params, "enabled");
    if ((ssid != NULL && (!cJSON_IsString(ssid) || strlen(ssid->valuestring) == 0 || strlen(ssid->valuestring) > 32))
        || (pass != NULL && !cJSON_IsString(pass)) || (enabled != NULL && !cJSON_IsBool(enabled))) {
        *message = "expected {\"ssid\": 1-32 bytes, \"password\": string, \"enabled\": bool}";
        return NULL;
    }
    size_t plen = cJSON_IsString(pass) ? strlen(pass->valuestring) : 0;
    if (plen != 0 && (plen < 8 || plen > 64)) {
        *message = "a WiFi password has 8 to 63 characters (or 64 hex digits)";
        return NULL;
    }
    lock();
    if (ssid == NULL && enabled != NULL && cJSON_IsTrue(enabled) && s_ssid[0] == '\0') {
        unlock();
        *message = "no network set: send ssid and password";
        return NULL;
    }
    if (ssid != NULL) {
        strlcpy(s_ssid, ssid->valuestring, sizeof(s_ssid));
        strlcpy(s_pass, cJSON_IsString(pass) ? pass->valuestring : "", sizeof(s_pass));
        s_enabled = true;
    }
    if (enabled != NULL) {
        s_enabled = cJSON_IsTrue(enabled);
    }
    bool need_key = !s_has_key && s_enabled;
    unlock();
    if (need_key) {
        // The radio is on by now when it can be, for the RNG's entropy.
        bring_up();
        uint8_t key[LINK_KEY_BYTES];
        esp_fill_random(key, sizeof(key));
        lock();
        memcpy(s_key, key, sizeof(key));
        s_has_key = true;
        unlock();
        memset(key, 0, sizeof(key));
    }
    lock();
    save();
    unlock();
    apply();
    cJSON *st = net_status();
    if (s_has_key) {
        add_key(st);
    }
    return st;
}

static cJSON *forget(void)
{
    lock();
    s_enabled = false;
    s_ssid[0] = '\0';
    s_pass[0] = '\0';
    memset(s_key, 0, sizeof(s_key));
    s_has_key = false;
    save();
    unlock();
    apply();
    return net_status();
}

cJSON *net_rpc(const char *method, const cJSON *params, bool usb, int *code, const char **message)
{
    *code = -32602;
    if (strcmp(method, "wifi/status") == 0) {
        return net_status();
    }
    if (strcmp(method, "wifi/scan") == 0) {
        return scan(message);
    }
    bool usb_only = strcmp(method, "wifi/set") == 0 || strcmp(method, "wifi/forget") == 0
                    || strcmp(method, "wifi/pair") == 0;
    if (!usb_only) {
        *code = -32601;
        *message = "method not found";
        return NULL;
    }
    if (!usb) {
        *code = -32001;
        *message = "only over USB";
        return NULL;
    }
    if (strcmp(method, "wifi/set") == 0) {
        return set(params, message);
    }
    if (strcmp(method, "wifi/forget") == 0) {
        return forget();
    }
    // wifi/pair: the key again, for another computer or a new install.
    if (!s_has_key) {
        *message = "the WiFi isn't set up";
        return NULL;
    }
    cJSON *pair = cJSON_CreateObject();
    cJSON_AddStringToObject(pair, "id", s_id);
    add_key(pair);
    return pair;
}
