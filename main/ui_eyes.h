#pragma once

/* Cartoon eyes, one per screen, for a voice conversation: they open over the
 * watch face on the wake word, look and move differently while listening,
 * thinking and speaking, and close again when it's over. Rounded-rect eyes
 * in the Cozmo/EMO style, drawn by LVGL each frame. */

#include "board_display.h"
#include "lvgl.h"
#include "voice.h"

#ifdef __cplusplus
extern "C" {
#endif

/** Call once, with the LVGL lock held. */
void ui_eyes_create(lv_display_t *const displays[BOARD_LCD_COUNT]);

/** VOICE_IDLE closes the eyes and brings the watch face back. With the LVGL
 * lock held. */
void ui_eyes_show(voice_state_t state);

/** Mic level while listening, speaker level while speaking, 0..1. With the
 * LVGL lock held. */
void ui_eyes_set_level(float level);

#ifdef __cplusplus
}
#endif
