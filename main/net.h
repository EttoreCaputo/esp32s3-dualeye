#pragma once

/* WiFi: the board joins the network set over USB (`wifi/set`), says where it
 * is with mDNS (_dualeye._tcp) and takes one host at a time on a TCP port.
 * The bytes go to the link task (link.c), which runs the same protocol on
 * them as on USB, sealed (docs/protocol.md, "WiFi"). Off until set up: then
 * nothing of the WiFi is started and it costs no RAM. */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "cJSON.h"
#include "esp_err.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "link_crypto.h"

#ifdef __cplusplus
extern "C" {
#endif

#define NET_TCP_PORT 7340
#define NET_CHUNK 512

/* What the TCP task hands the link task, in order. `gen` names the
 * connection: it changes with each host that connects. */
typedef struct {
    uint16_t gen;
    /* > 0: bytes received; NET_CONNECTED: a host connected; NET_CLOSED: it went. */
    int16_t len;
    uint8_t data[NET_CHUNK];
} net_chunk_t;

#define NET_CONNECTED 0
#define NET_CLOSED -1

/** Load the WiFi settings (NVS must be up) and join the network if set. */
void net_init(void);

/** Where the TCP task puts what it receives; NULL while the WiFi has never
 * been on since boot. */
QueueHandle_t net_rx_queue(void);

/** Write all of `buf` to connection `gen` within `wait_ms`. ESP_OK; ESP_ERR_TIMEOUT
 * when none of it could go (the connection is fine); ESP_FAIL when it's gone or
 * only part went, and then it's being closed. */
esp_err_t net_send(uint16_t gen, const uint8_t *buf, size_t len, uint32_t wait_ms);

/** Close connection `gen` (a NET_CLOSED chunk follows). */
void net_drop(uint16_t gen);

/** The pairing key; false if the WiFi was never set up. */
bool net_key(uint8_t key[LINK_KEY_BYTES]);

/** The board's id: its WiFi MAC address, 12 lowercase hex digits. */
const char *net_board_id(void);

/** The WiFi's state for get_state and wifi/status (no secrets). */
cJSON *net_status(void);

/** `wifi/...` requests (rpc.c). `usb`: the request came over USB, which may
 * change the WiFi and read the pairing key. Returns the result, or NULL with
 * `*code` and `*message` set. */
cJSON *net_rpc(const char *method, const cJSON *params, bool usb, int *code, const char **message);

#ifdef __cplusplus
}
#endif
