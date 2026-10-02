#pragma once

/* The cover art of what's playing on the host, for the music face: one
 * ART_SIZE x ART_SIZE RGB565 picture in PSRAM that both screens show. It is
 * not kept across a reboot (the host sends it again).
 *
 * The host sends it with `music/art` {"id", "offset", "data": base64}, at most
 * ART_CHUNK bytes at a time and in order, into a second buffer; the last chunk
 * copies it over the one on screen. The snapshot's `music.art` then names the
 * id, so a cover that hasn't arrived (yet) isn't shown for another track. */

#include <stdint.h>

#include "cJSON.h"
#include "lvgl.h"

#ifdef __cplusplus
extern "C" {
#endif

#define ART_SIZE 240
#define ART_BYTES (ART_SIZE * ART_SIZE * 2)
/* 40 chunks a cover; 3840 bytes of base64 leave room for the JSON in a frame. */
#define ART_CHUNK 2880

/** Allocate the buffers. */
void art_init(void);

/** The id of the cover on screen; 0 without one. */
uint32_t art_id(void);

/** The cover, for lv_image_set_src(); its pixels change in place when a new
 * one arrives (art_generation() moves on). With the LVGL lock held. */
const lv_image_dsc_t *art_image(void);

/** Bumped whenever a new cover is in. */
uint32_t art_generation(void);

/** The `music/...` RPC methods: a result, or NULL with `message` set. */
cJSON *art_rpc(const char *method, const cJSON *params, const char **message);

#ifdef __cplusplus
}
#endif
