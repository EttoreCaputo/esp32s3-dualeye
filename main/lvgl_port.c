#include "lvgl_port.h"

#include <sys/lock.h>

#include "esp_check.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#define LVGL_DRAW_BUF_LINES 40
#define LVGL_TICK_PERIOD_MS 2
/* 6 KB left about 1 KB free (M0); the toast and later overlays need more. */
#define LVGL_TASK_STACK_SIZE (8 * 1024)
#define LVGL_TASK_PRIORITY 5

static const char *TAG = "lvgl_port";
static _lock_t s_lvgl_lock;

/* Filled by the LVGL task: the window being measured, and the last full one. */
static int64_t s_window_start_us;
static int64_t s_window_busy_us;
static uint32_t s_window_max_us;
static lvgl_port_stats_t s_stats;
static portMUX_TYPE s_stats_mux = portMUX_INITIALIZER_UNLOCKED;

static void account(int64_t start_us, int64_t end_us)
{
    uint32_t took = (uint32_t) (end_us - start_us);
    s_window_busy_us += took;
    if (took > s_window_max_us) {
        s_window_max_us = took;
    }
    int64_t span = end_us - s_window_start_us;
    if (span >= LVGL_PORT_STATS_WINDOW_MS * 1000) {
        portENTER_CRITICAL(&s_stats_mux);
        s_stats.busy_pct = 100.0f * (float) s_window_busy_us / (float) span;
        s_stats.max_us = s_window_max_us;
        portEXIT_CRITICAL(&s_stats_mux);
        s_window_start_us = end_us;
        s_window_busy_us = 0;
        s_window_max_us = 0;
    }
}

void lvgl_port_get_stats(lvgl_port_stats_t *out)
{
    portENTER_CRITICAL(&s_stats_mux);
    *out = s_stats;
    portEXIT_CRITICAL(&s_stats_mux);
}

static bool notify_flush_ready(esp_lcd_panel_io_handle_t panel_io,
                               esp_lcd_panel_io_event_data_t *edata, void *user_ctx)
{
    lv_display_t *disp = (lv_display_t *) user_ctx;
    lv_display_flush_ready(disp);
    return false;
}

static void flush_cb(lv_display_t *disp, const lv_area_t *area, uint8_t *px_map)
{
    esp_lcd_panel_handle_t panel = (esp_lcd_panel_handle_t) lv_display_get_user_data(disp);
    const int x1 = area->x1;
    const int x2 = area->x2;
    const int y1 = area->y1;
    const int y2 = area->y2;
    const int w = x2 - x1 + 1;
    const int h = y2 - y1 + 1;

    // SPI LCD expects big-endian RGB565 on the wire.
    lv_draw_sw_rgb565_swap(px_map, w * h);
    esp_lcd_panel_draw_bitmap(panel, x1, y1, x2 + 1, y2 + 1, px_map);
}

static void tick_cb(void *arg)
{
    lv_tick_inc(LVGL_TICK_PERIOD_MS);
}

static void lvgl_task(void *arg)
{
    ESP_LOGI(TAG, "LVGL task started");
    const uint32_t min_delay_ms = 1000 / configTICK_RATE_HZ;
    s_window_start_us = esp_timer_get_time();
    while (true) {
        _lock_acquire(&s_lvgl_lock);
        int64_t start = esp_timer_get_time();
        uint32_t delay_ms = lv_timer_handler();
        account(start, esp_timer_get_time());
        _lock_release(&s_lvgl_lock);
        if (delay_ms < min_delay_ms) {
            delay_ms = min_delay_ms;
        } else if (delay_ms > 500) {
            delay_ms = 500;
        }
        vTaskDelay(pdMS_TO_TICKS(delay_ms));
    }
}

void lvgl_port_lock(void)
{
    _lock_acquire(&s_lvgl_lock);
}

void lvgl_port_unlock(void)
{
    _lock_release(&s_lvgl_lock);
}

static lv_display_t *register_display(const board_lcd_t *lcd)
{
    lv_display_t *disp = lv_display_create(BOARD_LCD_H_RES, BOARD_LCD_V_RES);
    const size_t buf_size = BOARD_LCD_H_RES * LVGL_DRAW_BUF_LINES * sizeof(lv_color16_t);

    void *buf1 = spi_bus_dma_memory_alloc(BOARD_LCD_SPI_HOST, buf_size, 0);
    void *buf2 = spi_bus_dma_memory_alloc(BOARD_LCD_SPI_HOST, buf_size, 0);
    ESP_ERROR_CHECK(buf1 && buf2 ? ESP_OK : ESP_ERR_NO_MEM);

    lv_display_set_buffers(disp, buf1, buf2, buf_size, LV_DISPLAY_RENDER_MODE_PARTIAL);
    lv_display_set_user_data(disp, lcd->panel);
    lv_display_set_color_format(disp, LV_COLOR_FORMAT_RGB565);
    lv_display_set_flush_cb(disp, flush_cb);

    const esp_lcd_panel_io_callbacks_t cbs = {
        .on_color_trans_done = notify_flush_ready,
    };
    ESP_ERROR_CHECK(esp_lcd_panel_io_register_event_callbacks(lcd->io, &cbs, disp));
    return disp;
}

esp_err_t lvgl_port_init(const board_lcd_t lcds[BOARD_LCD_COUNT],
                         lv_display_t *out_displays[BOARD_LCD_COUNT])
{
    lv_init();

    out_displays[UI_SCREEN_CPU] = register_display(&lcds[UI_SCREEN_CPU]);
    out_displays[UI_SCREEN_GPU] = register_display(&lcds[UI_SCREEN_GPU]);

    const esp_timer_create_args_t tick_args = {
        .callback = &tick_cb,
        .name = "lvgl_tick",
    };
    esp_timer_handle_t tick_timer = NULL;
    ESP_ERROR_CHECK(esp_timer_create(&tick_args, &tick_timer));
    ESP_ERROR_CHECK(esp_timer_start_periodic(tick_timer, LVGL_TICK_PERIOD_MS * 1000));

    // Core 0: core 1 runs the audio front end and the wake word.
    BaseType_t ok = xTaskCreatePinnedToCore(lvgl_task, "lvgl", LVGL_TASK_STACK_SIZE, NULL,
                                            LVGL_TASK_PRIORITY, NULL, 0);
    ESP_RETURN_ON_ERROR(ok == pdPASS ? ESP_OK : ESP_ERR_NO_MEM, TAG, "task create failed");

    ESP_LOGI(TAG, "Dual LVGL displays registered");
    return ESP_OK;
}
