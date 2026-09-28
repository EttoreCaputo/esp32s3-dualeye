#include <string.h>

#include "audio_selftest.h"
#include "board_display.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lvgl_port.h"
#include "metrics_io.h"
#include "metrics_model.h"
#include "nvs.h"
#include "nvs_flash.h"
#include "ui_watch.h"

static const char *TAG = "dualeye";

static board_lcd_t s_lcds[BOARD_LCD_COUNT];
static lv_display_t *s_displays[BOARD_LCD_COUNT];
/* Rotation each screen is drawn with. Kept in NVS, so the board boots the way
 * the host last turned it instead of waiting upright for the first line. */
static uint16_t s_rot[BOARD_LCD_COUNT];

#define NVS_NAMESPACE "dualeye"

static const char *const ROT_KEYS[BOARD_LCD_COUNT] = {
    [UI_SCREEN_CPU] = "rot_cpu",
    [UI_SCREEN_GPU] = "rot_gpu",
};

static void nvs_start(void)
{
    esp_err_t err = nvs_flash_init();
    if (err == ESP_ERR_NVS_NO_FREE_PAGES || err == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        ESP_ERROR_CHECK(nvs_flash_erase());
        err = nvs_flash_init();
    }
    if (err != ESP_OK) {
        ESP_LOGW(TAG, "NVS unavailable (%s), rotation won't survive a reboot", esp_err_to_name(err));
    }
}

static void load_rotation(void)
{
    nvs_handle_t nvs;
    if (nvs_open(NVS_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return;
    }
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        uint16_t rot = 0;
        if (nvs_get_u16(nvs, ROT_KEYS[i], &rot) == ESP_OK && rot % 90 == 0 && rot < 360) {
            s_rot[i] = rot;
        }
    }
    nvs_close(nvs);
}

static void save_rotation(void)
{
    nvs_handle_t nvs;
    if (nvs_open(NVS_NAMESPACE, NVS_READWRITE, &nvs) != ESP_OK) {
        return;
    }
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        nvs_set_u16(nvs, ROT_KEYS[i], s_rot[i]);
    }
    nvs_commit(nvs);
    nvs_close(nvs);
}

/** Call with the LVGL lock held: no frame is being drawn meanwhile. */
static void apply_rotation(const uint16_t want[BOARD_LCD_COUNT])
{
    bool changed = false;
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        if (want[i] == s_rot[i]) {
            continue;
        }
        if (board_display_rotate(s_lcds[i].panel, i, (board_lcd_rotation_t) want[i]) != ESP_OK) {
            continue;
        }
        s_rot[i] = want[i];
        changed = true;
        // Everything on the panel was drawn the old way round.
        lv_obj_invalidate(lv_display_get_screen_active(s_displays[i]));
        ESP_LOGI(TAG, "screen %d turned %u deg", i, want[i]);
    }
    if (changed) {
        save_rotation();
    }
}

static void ui_refresh_task(void *arg)
{
    metrics_snapshot_t prev;
    memset(&prev, 0, sizeof(prev));

    while (true) {
        metrics_snapshot_t snap;
        metrics_model_get(&snap);
        if (memcmp(&prev, &snap, sizeof(snap)) != 0) {
            lvgl_port_lock();
            // Until the host's first line, keep the rotation loaded at boot.
            if (snap.state != METRICS_UI_WAITING) {
                const uint16_t want[BOARD_LCD_COUNT] = {
                    [UI_SCREEN_CPU] = snap.cpu_rot,
                    [UI_SCREEN_GPU] = snap.gpu_rot,
                };
                apply_rotation(want);
            }
            ui_watch_update(&snap);
            lvgl_port_unlock();
            prev = snap;
        }
        vTaskDelay(pdMS_TO_TICKS(200));
    }
}

void app_main(void)
{
    nvs_start();
    ESP_ERROR_CHECK(board_display_init(s_lcds));
    load_rotation();
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        ESP_ERROR_CHECK(board_display_rotate(s_lcds[i].panel, i, (board_lcd_rotation_t) s_rot[i]));
    }

    metrics_model_init();

    ESP_ERROR_CHECK(lvgl_port_init(s_lcds, s_displays));

    lvgl_port_lock();
    ui_watch_create(s_displays[UI_SCREEN_CPU], s_displays[UI_SCREEN_GPU]);

    metrics_snapshot_t snap;
    metrics_model_get(&snap);
    ui_watch_update(&snap);
    lvgl_port_unlock();

    board_display_set_backlight(true);

    BaseType_t ui_ok = xTaskCreate(ui_refresh_task, "ui_refresh", 4096, NULL, 4, NULL);
    ESP_ERROR_CHECK(ui_ok == pdPASS ? ESP_OK : ESP_ERR_NO_MEM);
    audio_selftest_start();
    metrics_io_start();
    // A host already listening (e.g. right after flashing) learns the version without asking.
    metrics_io_report_version();
    ESP_LOGI(TAG, "Watch UI ready, waiting for USB metrics");
}
