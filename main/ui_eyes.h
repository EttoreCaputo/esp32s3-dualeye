#pragma once

/* Cartoon eyes, one per screen, for a voice conversation: they open over the
 * watch face on the wake word, look and move differently while listening,
 * thinking and speaking, and close again when it's over. While nobody is
 * talking they also open now and then on their own for a short scene (a
 * wink, a yawn, a look around...). Rounded-rect eyes in the Cozmo/EMO style,
 * drawn by LVGL each frame. */

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

/** The scenes while idle, at random every minute or two: on or off. With the
 * LVGL lock held. */
void ui_eyes_set_idle(bool on);

/** Something else is on the screens for the voice (the ring): no scenes, and
 * one playing is gone at once. With the LVGL lock held. */
void ui_eyes_set_busy(bool busy);

/** Play the scene `name` now (NULL: any). False when it's unknown, or a
 * conversation is on. With the LVGL lock held. */
bool ui_eyes_play(const char *name);

/** The scenes' names, up to `max`; returns how many. */
int ui_eyes_animations(const char **names, int max);

#ifdef __cplusplus
}
#endif
