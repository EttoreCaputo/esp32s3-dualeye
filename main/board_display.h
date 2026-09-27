#pragma once

#include <stdbool.h>

#include "esp_err.h"
#include "esp_lcd_panel_io.h"
#include "esp_lcd_panel_ops.h"
#include "driver/spi_master.h"

#ifdef __cplusplus
extern "C" {
#endif

#define BOARD_LCD_H_RES 240
#define BOARD_LCD_V_RES 240
#define BOARD_LCD_SPI_HOST SPI2_HOST
#define BOARD_LCD_SPI_CLOCK_HZ (40 * 1000 * 1000)
#define BOARD_LCD_COUNT 2

#define UI_SCREEN_CPU 0
#define UI_SCREEN_GPU 1

/** Physical mounting: left eye CW, right eye CCW (DualEye). */
typedef enum {
    BOARD_LCD_ROT_0 = 0,
    BOARD_LCD_ROT_90_CW = 90,
    BOARD_LCD_ROT_180 = 180,
    BOARD_LCD_ROT_90_CCW = 270,
} board_lcd_rotation_t;

typedef struct {
    esp_lcd_panel_handle_t panel;
    esp_lcd_panel_io_handle_t io;
} board_lcd_t;

esp_err_t board_display_init(board_lcd_t out_lcds[BOARD_LCD_COUNT]);
esp_err_t board_display_set_rotation(esp_lcd_panel_handle_t panel, board_lcd_rotation_t rot);
/** Turn screen `screen` (UI_SCREEN_*) by `extra` more degrees clockwise than
 * its DualEye mounting. Takes effect on the next frame drawn. */
esp_err_t board_display_rotate(esp_lcd_panel_handle_t panel, int screen, board_lcd_rotation_t extra);
void board_display_set_backlight(bool on);

#ifdef __cplusplus
}
#endif
