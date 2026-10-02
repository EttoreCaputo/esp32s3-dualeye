#pragma once

/* A picture or an animation for each screen's image face, uploaded by the
 * host and kept in the `media` partition: one slot per screen.
 *
 * The host does the decoding: what it sends is already 240 x 240 RGB565, a
 * frame at a time, each raw or run-length coded. A slot holds
 *
 *   header     "DEIM", version 1, flags, frames, width, height (16 bytes)
 *   frames  x  offset u32, length u32, delay_ms u16, coding u8, 0 (12 bytes)
 *   data       the frames, at their offsets (from the start of the slot)
 *
 * all little-endian. Coding 0 is raw pixels; coding 1 is u16 words: with the
 * top bit set, (word & 0x7FFF) + 1 copies of the pixel that follows; without
 * it, word + 1 pixels as they are.
 *
 * Uploads come as `media/begin`, `media/write`... and `media/end`, checked
 * against a CRC-32; only then does NVS mark the slot as holding an image. */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "cJSON.h"
#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

#define MEDIA_WIDTH 240
#define MEDIA_HEIGHT 240
#define MEDIA_CODING_RAW 0
#define MEDIA_CODING_RLE 1

typedef struct {
    uint32_t offset;
    uint32_t length;
    uint16_t delay_ms;
    uint8_t coding;
    uint8_t reserved;
} media_frame_t;

/** Find the partition and the images kept in it. After NVS is up. */
void media_init(void);

/** The slot of `screen` holds a checked image. */
bool media_present(int screen);

/** Frames in the image of `screen`; 0 without one. */
int media_frame_count(int screen);

/** Frame `index` of `screen`'s image, decoded into `out` (MEDIA_WIDTH x
 * MEDIA_HEIGHT RGB565). Its delay in `delay_ms`. */
bool media_decode(int screen, int index, uint16_t *out, uint16_t *delay_ms);

/** Bumped whenever an image appears or goes, so the UI reloads. */
uint32_t media_generation(void);

/** The `media/...` RPC methods: a result, or NULL with `message` set. */
cJSON *media_rpc(const char *method, const cJSON *params, const char **message);

#ifdef __cplusplus
}
#endif
