#pragma once

/* The "eyes" overlay: a ring round the edge of both screens that shows the
 * voice state (listening, thinking, speaking, error) over the watch face. */

#include "board_display.h"
#include "lvgl.h"
#include "voice.h"

#ifdef __cplusplus
extern "C" {
#endif

/** Call once, with the LVGL lock held, before ui_toast_create() so a toast
 * stays on top. */
void ui_voice_create(lv_display_t *const displays[BOARD_LCD_COUNT]);

/** With the LVGL lock held. */
void ui_voice_show(voice_state_t state);

/** Mic level while listening, speaker level while speaking, 0..1: a brighter arc at the top of the ring
 * that grows with it. With the LVGL lock held. */
void ui_voice_set_level(float level);

#ifdef __cplusplus
}
#endif
