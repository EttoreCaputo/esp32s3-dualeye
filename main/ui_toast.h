#pragma once

/* A short message over the watch face, on one or both screens. */

#include <stdint.h>

#include "board_display.h"
#include "lvgl.h"

#ifdef __cplusplus
extern "C" {
#endif

#define UI_TOAST_TEXT_MAX 120

/** Call once, with the LVGL lock held. */
void ui_toast_create(lv_display_t *const displays[BOARD_LCD_COUNT]);

/** Show `text` (UTF-8; letters the font lacks are shown without accents or
 * as '?') on screen `screen` (UI_SCREEN_*) for `ms`, replacing any toast
 * already there. With the LVGL lock held. */
void ui_toast_show(int screen, const char *text, uint32_t ms);

#ifdef __cplusplus
}
#endif
