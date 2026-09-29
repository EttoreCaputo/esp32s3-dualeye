#pragma once

/* The "eyes" overlay: a ring round the edge of both screens that shows the
 * voice state (listening, thinking, speaking) over the watch face. */

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

#ifdef __cplusplus
}
#endif
