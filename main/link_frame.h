#pragma once

/* Protocol v2 frame codec (docs/protocol.md). Plain C with no ESP-IDF
 * dependency, so it also builds on the host for tests. */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define LINK_MAX_PAYLOAD 4096
/* Channel byte, u16 length, CRC. */
#define LINK_FRAME_OVERHEAD 5
/* COBS adds one byte per 254, plus one; then a delimiter on each side. */
#define LINK_ENCODED_MAX(payload) \
    ((payload) + LINK_FRAME_OVERHEAD + ((payload) + LINK_FRAME_OVERHEAD) / 254 + 1 + 2)

typedef enum {
    LINK_CHAN_CTRL = 0,
    LINK_CHAN_METRICS = 1,
    LINK_CHAN_LOG = 2,
    LINK_CHAN_AUDIO_UP = 3,
    LINK_CHAN_AUDIO_DOWN = 4,
} link_chan_t;

/** CRC-16/CCITT-FALSE: poly 0x1021, init 0xFFFF. */
uint16_t link_crc16(uint16_t crc, const uint8_t *data, size_t len);

/** Encode one frame, delimiters included, into `out` (at least
 * LINK_ENCODED_MAX(len) bytes). Returns the bytes written. */
size_t link_frame_encode(uint8_t chan, const uint8_t *payload, size_t len, uint8_t *out);

/** Decode one piece of the stream between two 0x00 delimiters, in place.
 * On success `*chan`, `*payload` (pointing into `buf`) and `*len` are set.
 * Fails on bad COBS, a length mismatch or a CRC mismatch. */
bool link_frame_decode(uint8_t *buf, size_t n, uint8_t *chan, uint8_t **payload, size_t *len);

#ifdef __cplusplus
}
#endif
