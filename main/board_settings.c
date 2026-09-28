#include "board_settings.h"

#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "nvs.h"
#include "nvs_flash.h"

#define NVS_NAMESPACE "dualeye"

static const char *TAG = "settings";

/* rot_* predate protocol v2: a board updated from 0.3 keeps its rotation. */
static const char *const ROT_KEYS[BOARD_LCD_COUNT] = {[UI_SCREEN_CPU] = "rot_cpu", [UI_SCREEN_GPU] = "rot_gpu"};
static const char *const FACE_KEYS[BOARD_LCD_COUNT] = {[UI_SCREEN_CPU] = "face_cpu", [UI_SCREEN_GPU] = "face_gpu"};
static const char *const BL_KEYS[BOARD_LCD_COUNT] = {[UI_SCREEN_CPU] = "bl_cpu", [UI_SCREEN_GPU] = "bl_gpu"};

static SemaphoreHandle_t s_lock;
static board_settings_t s_settings;

static bool valid_screen(int screen)
{
    return screen >= 0 && screen < BOARD_LCD_COUNT;
}

static void load(void)
{
    nvs_handle_t nvs;
    if (nvs_open(NVS_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return;
    }
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        uint16_t rot = 0;
        if (nvs_get_u16(nvs, ROT_KEYS[i], &rot) == ESP_OK && rot % 90 == 0 && rot < 360) {
            s_settings.rot[i] = rot;
        }
        uint8_t face = 0;
        if (nvs_get_u8(nvs, FACE_KEYS[i], &face) == ESP_OK && face < METRICS_FACE_COUNT) {
            s_settings.face[i] = (metrics_face_t) face;
        }
        uint8_t bl = 0;
        if (nvs_get_u8(nvs, BL_KEYS[i], &bl) == ESP_OK && bl <= 100) {
            s_settings.brightness[i] = bl;
        }
    }
    nvs_close(nvs);
}

static void save(const char *key, uint16_t value, bool wide)
{
    nvs_handle_t nvs;
    if (nvs_open(NVS_NAMESPACE, NVS_READWRITE, &nvs) != ESP_OK) {
        return;
    }
    esp_err_t err = wide ? nvs_set_u16(nvs, key, value) : nvs_set_u8(nvs, key, (uint8_t) value);
    if (err == ESP_OK) {
        err = nvs_commit(nvs);
    }
    nvs_close(nvs);
    if (err != ESP_OK) {
        ESP_LOGW(TAG, "saving %s failed: %s", key, esp_err_to_name(err));
    }
}

void board_settings_init(void)
{
    s_lock = xSemaphoreCreateMutex();
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        s_settings.brightness[i] = BOARD_BRIGHTNESS_DEFAULT;
    }
    esp_err_t err = nvs_flash_init();
    if (err == ESP_ERR_NVS_NO_FREE_PAGES || err == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        ESP_ERROR_CHECK(nvs_flash_erase());
        err = nvs_flash_init();
    }
    if (err != ESP_OK) {
        ESP_LOGW(TAG, "NVS unavailable (%s), settings won't survive a reboot", esp_err_to_name(err));
        return;
    }
    load();
}

void board_settings_get(board_settings_t *out)
{
    xSemaphoreTake(s_lock, portMAX_DELAY);
    *out = s_settings;
    xSemaphoreGive(s_lock);
}

esp_err_t board_settings_set_face(int screen, metrics_face_t face)
{
    if (!valid_screen(screen) || face >= METRICS_FACE_COUNT) {
        return ESP_ERR_INVALID_ARG;
    }
    xSemaphoreTake(s_lock, portMAX_DELAY);
    bool changed = s_settings.face[screen] != face;
    s_settings.face[screen] = face;
    xSemaphoreGive(s_lock);
    if (changed) {
        save(FACE_KEYS[screen], face, false);
    }
    return ESP_OK;
}

esp_err_t board_settings_set_rotation(int screen, uint16_t degrees)
{
    if (!valid_screen(screen) || degrees % 90 != 0 || degrees >= 360) {
        return ESP_ERR_INVALID_ARG;
    }
    xSemaphoreTake(s_lock, portMAX_DELAY);
    bool changed = s_settings.rot[screen] != degrees;
    s_settings.rot[screen] = degrees;
    xSemaphoreGive(s_lock);
    if (changed) {
        save(ROT_KEYS[screen], degrees, true);
    }
    return ESP_OK;
}

esp_err_t board_settings_set_brightness(int screen, uint8_t percent)
{
    if (!valid_screen(screen) || percent > 100) {
        return ESP_ERR_INVALID_ARG;
    }
    esp_err_t err = board_display_set_brightness(screen, percent);
    if (err != ESP_OK) {
        return err;
    }
    xSemaphoreTake(s_lock, portMAX_DELAY);
    bool changed = s_settings.brightness[screen] != percent;
    s_settings.brightness[screen] = percent;
    xSemaphoreGive(s_lock);
    if (changed) {
        save(BL_KEYS[screen], percent, false);
    }
    return ESP_OK;
}
