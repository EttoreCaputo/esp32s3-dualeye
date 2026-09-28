#pragma once

/* USB link to the host, protocol v2 (docs/protocol.md): frames over the
 * USB Serial/JTAG driver in binary mode, ESP_LOG redirected into `log` frames. */

#include <stddef.h>
#include <stdint.h>

#include "esp_err.h"
#include "link_frame.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    uint32_t rx_frames;
    uint32_t rx_bad;       /* pieces that weren't a valid frame */
    uint32_t tx_frames;
    uint32_t tx_dropped;   /* host not reading, or the TX buffer full */
} link_stats_t;

/** Install the driver and redirect ESP_LOG. Call first thing in app_main:
 * anything logged before goes out as raw console text. */
esp_err_t link_init(void);

/** Called from the link task with each frame received on a channel. The
 * payload is followed by a NUL, so JSON can be parsed in place. */
typedef void (*link_rx_cb_t)(uint8_t *payload, size_t len);

void link_on_receive(link_chan_t chan, link_rx_cb_t cb);

/** Start the reader task. Register the receive callbacks first. */
void link_start(void);

/** Send one frame. Safe from any task; drops it (and returns an error) if the
 * host doesn't take it within a short wait, or at once for a log line while
 * the host isn't reading. */
esp_err_t link_send(link_chan_t chan, const void *payload, size_t len);

/** link_send with an explicit wait for room in the TX buffer. */
esp_err_t link_send_timeout(link_chan_t chan, const void *payload, size_t len, uint32_t wait_ms);

/** printf a line (no trailing newline needed) to the host as a `log` frame. */
void link_log_printf(const char *fmt, ...) __attribute__((format(printf, 1, 2)));

void link_get_stats(link_stats_t *out);

#ifdef __cplusplus
}
#endif
