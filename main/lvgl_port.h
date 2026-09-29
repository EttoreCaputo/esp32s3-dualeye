#pragma once

#include "board_display.h"
#include "esp_err.h"
#include "lvgl.h"

#ifdef __cplusplus
extern "C" {
#endif

esp_err_t lvgl_port_init(const board_lcd_t lcds[BOARD_LCD_COUNT],
                         lv_display_t *out_displays[BOARD_LCD_COUNT]);

void lvgl_port_lock(void);
void lvgl_port_unlock(void);

/** How hard LVGL worked over the last full window of LVGL_PORT_STATS_WINDOW_MS:
 * the share of time in lv_timer_handler() (rendering, and waiting for the SPI
 * flush when both draw buffers are full) and its longest single run. */
#define LVGL_PORT_STATS_WINDOW_MS 5000
typedef struct {
    float busy_pct;
    uint32_t max_us;
} lvgl_port_stats_t;

void lvgl_port_get_stats(lvgl_port_stats_t *out);

#ifdef __cplusplus
}
#endif
