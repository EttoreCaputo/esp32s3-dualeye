#include "board_display.h"

#include "driver/gpio.h"
#include "driver/ledc.h"
#include "esp_check.h"
#include "esp_lcd_gc9a01.h"
#include "esp_log.h"

#define LCD_SCLK_GPIO 41
#define LCD_MOSI_GPIO 42
#define LCD_DC_GPIO 45

#define LCD1_CS_GPIO 47
#define LCD1_RST_GPIO 48
#define LCD1_BL_GPIO 46

#define LCD2_CS_GPIO 38
#define LCD2_RST_GPIO 8
#define LCD2_BL_GPIO 39

static const char *TAG = "board_display";

// DualEye mounting: left eye 90° CCW, right eye 90° CW.
static const board_lcd_rotation_t s_mounting[BOARD_LCD_COUNT] = {
    [UI_SCREEN_CPU] = BOARD_LCD_ROT_90_CCW,
    [UI_SCREEN_GPU] = BOARD_LCD_ROT_90_CW,
};

static esp_err_t init_one(int cs_gpio, int rst_gpio, board_lcd_t *lcd)
{
    const esp_lcd_panel_io_spi_config_t io_config = {
        .cs_gpio_num = cs_gpio,
        .dc_gpio_num = LCD_DC_GPIO,
        .spi_mode = 0,
        .pclk_hz = BOARD_LCD_SPI_CLOCK_HZ,
        .trans_queue_depth = 10,
        .lcd_cmd_bits = 8,
        .lcd_param_bits = 8,
    };
    ESP_RETURN_ON_ERROR(esp_lcd_new_panel_io_spi((esp_lcd_spi_bus_handle_t) BOARD_LCD_SPI_HOST,
                                                 &io_config, &lcd->io),
                        TAG, "panel io failed");

    const esp_lcd_panel_dev_config_t panel_config = {
        .reset_gpio_num = rst_gpio,
        .rgb_ele_order = LCD_RGB_ELEMENT_ORDER_BGR,
        .bits_per_pixel = 16,
    };
    ESP_RETURN_ON_ERROR(esp_lcd_new_panel_gc9a01(lcd->io, &panel_config, &lcd->panel),
                        TAG, "gc9a01 failed");
    ESP_RETURN_ON_ERROR(esp_lcd_panel_reset(lcd->panel), TAG, "reset failed");
    ESP_RETURN_ON_ERROR(esp_lcd_panel_init(lcd->panel), TAG, "init failed");
    // Required for GC9A01 panels used on Waveshare DualEye boards.
    ESP_RETURN_ON_ERROR(esp_lcd_panel_invert_color(lcd->panel, true), TAG, "invert failed");
    ESP_RETURN_ON_ERROR(esp_lcd_panel_disp_on_off(lcd->panel, true), TAG, "disp on failed");
    return ESP_OK;
}

esp_err_t board_display_set_rotation(esp_lcd_panel_handle_t panel, board_lcd_rotation_t rot)
{
    // MADCTL mapping matches Espressif GC9A01 + LVGL examples (ROT_0 = DualEye default).
    switch (rot) {
    case BOARD_LCD_ROT_0:
        ESP_RETURN_ON_ERROR(esp_lcd_panel_swap_xy(panel, false), TAG, "swap_xy");
        ESP_RETURN_ON_ERROR(esp_lcd_panel_mirror(panel, true, false), TAG, "mirror");
        break;
    case BOARD_LCD_ROT_90_CW:
        ESP_RETURN_ON_ERROR(esp_lcd_panel_swap_xy(panel, true), TAG, "swap_xy");
        ESP_RETURN_ON_ERROR(esp_lcd_panel_mirror(panel, true, true), TAG, "mirror");
        break;
    case BOARD_LCD_ROT_180:
        ESP_RETURN_ON_ERROR(esp_lcd_panel_swap_xy(panel, false), TAG, "swap_xy");
        ESP_RETURN_ON_ERROR(esp_lcd_panel_mirror(panel, false, true), TAG, "mirror");
        break;
    case BOARD_LCD_ROT_90_CCW:
        ESP_RETURN_ON_ERROR(esp_lcd_panel_swap_xy(panel, true), TAG, "swap_xy");
        ESP_RETURN_ON_ERROR(esp_lcd_panel_mirror(panel, false, false), TAG, "mirror");
        break;
    default:
        return ESP_ERR_INVALID_ARG;
    }
    return ESP_OK;
}

esp_err_t board_display_rotate(esp_lcd_panel_handle_t panel, int screen, board_lcd_rotation_t extra)
{
    if (screen < 0 || screen >= BOARD_LCD_COUNT || extra % 90 != 0) {
        return ESP_ERR_INVALID_ARG;
    }
    return board_display_set_rotation(panel, (board_lcd_rotation_t) ((s_mounting[screen] + extra) % 360));
}

/* Backlight PWM, one channel per screen. 20 kHz stays above hearing, which
 * matters with a microphone on the board. */
#define BL_LEDC_TIMER LEDC_TIMER_0
#define BL_LEDC_FREQ_HZ 20000
#define BL_LEDC_RES LEDC_TIMER_10_BIT
#define BL_LEDC_MAX ((1 << 10) - 1)

static const int s_bl_gpio[BOARD_LCD_COUNT] = {
    [UI_SCREEN_CPU] = LCD1_BL_GPIO,
    [UI_SCREEN_GPU] = LCD2_BL_GPIO,
};

static esp_err_t backlight_init(void)
{
    const ledc_timer_config_t timer = {
        .speed_mode = LEDC_LOW_SPEED_MODE,
        .duty_resolution = BL_LEDC_RES,
        .timer_num = BL_LEDC_TIMER,
        .freq_hz = BL_LEDC_FREQ_HZ,
        .clk_cfg = LEDC_AUTO_CLK,
    };
    ESP_RETURN_ON_ERROR(ledc_timer_config(&timer), TAG, "backlight timer");
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        const ledc_channel_config_t ch = {
            .gpio_num = s_bl_gpio[i],
            .speed_mode = LEDC_LOW_SPEED_MODE,
            .channel = (ledc_channel_t) i,
            .timer_sel = BL_LEDC_TIMER,
            .duty = 0,
        };
        ESP_RETURN_ON_ERROR(ledc_channel_config(&ch), TAG, "backlight channel");
    }
    return ESP_OK;
}

esp_err_t board_display_set_brightness(int screen, int percent)
{
    if (screen < 0 || screen >= BOARD_LCD_COUNT || percent < 0 || percent > 100) {
        return ESP_ERR_INVALID_ARG;
    }
    // Perceived brightness is roughly quadratic in duty.
    uint32_t duty = (uint32_t) (BL_LEDC_MAX * percent * percent / 10000);
    if (percent > 0 && duty == 0) {
        duty = 1;
    }
    ESP_RETURN_ON_ERROR(ledc_set_duty(LEDC_LOW_SPEED_MODE, (ledc_channel_t) screen, duty), TAG, "duty");
    return ledc_update_duty(LEDC_LOW_SPEED_MODE, (ledc_channel_t) screen);
}

esp_err_t board_display_init(board_lcd_t out_lcds[BOARD_LCD_COUNT])
{
    // Dark until the first frame is drawn.
    ESP_RETURN_ON_ERROR(backlight_init(), TAG, "backlight failed");

    // Partial LVGL buffers are ~60 lines; keep headroom for DMA transfers.
    const spi_bus_config_t bus_config = {
        .sclk_io_num = LCD_SCLK_GPIO,
        .mosi_io_num = LCD_MOSI_GPIO,
        .miso_io_num = -1,
        .quadwp_io_num = -1,
        .quadhd_io_num = -1,
        .max_transfer_sz = BOARD_LCD_H_RES * 80 * sizeof(uint16_t) + 8,
    };
    ESP_RETURN_ON_ERROR(spi_bus_initialize(BOARD_LCD_SPI_HOST, &bus_config, SPI_DMA_CH_AUTO),
                        TAG, "spi bus failed");

    ESP_RETURN_ON_ERROR(init_one(LCD1_CS_GPIO, LCD1_RST_GPIO, &out_lcds[UI_SCREEN_CPU]),
                        TAG, "LCD1 failed");
    ESP_RETURN_ON_ERROR(init_one(LCD2_CS_GPIO, LCD2_RST_GPIO, &out_lcds[UI_SCREEN_GPU]),
                        TAG, "LCD2 failed");

    ESP_RETURN_ON_ERROR(board_display_rotate(out_lcds[UI_SCREEN_CPU].panel, UI_SCREEN_CPU,
                                             BOARD_LCD_ROT_0),
                        TAG, "LCD1 rotation failed");
    ESP_RETURN_ON_ERROR(board_display_rotate(out_lcds[UI_SCREEN_GPU].panel, UI_SCREEN_GPU,
                                             BOARD_LCD_ROT_0),
                        TAG, "LCD2 rotation failed");

    ESP_LOGI(TAG, "Dual GC9A01 ready (L:+90 CCW, R:+90 CW)");
    return ESP_OK;
}
