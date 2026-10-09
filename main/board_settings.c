#include "board_settings.h"

#include <string.h>

#include "board_audio.h"
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
static const char *const SRC_KEYS[BOARD_LCD_COUNT] = {[UI_SCREEN_CPU] = "src_cpu", [UI_SCREEN_GPU] = "src_gpu"};

#define MIC_MUTED_KEY "mic_muted"
#define WAKE_WORD_KEY "wake_word"
#define VOLUME_KEY "volume"
#define EYES_KEY "eyes"
#define IDLE_EYES_KEY "idle_eyes"
#define WAKE_SOUND_KEY "wake_sound"
#define PET_SOUNDS_KEY "pet_sounds"
#define PET_REACT_KEY "pet_react"

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
        uint8_t src = 0;
        if (nvs_get_u8(nvs, SRC_KEYS[i], &src) == ESP_OK && src < METRICS_SOURCE_COUNT) {
            s_settings.source[i] = (metrics_source_t) src;
        }
        uint8_t bl = 0;
        if (nvs_get_u8(nvs, BL_KEYS[i], &bl) == ESP_OK && bl <= 100) {
            s_settings.brightness[i] = bl;
        }
    }
    uint8_t muted = 0;
    if (nvs_get_u8(nvs, MIC_MUTED_KEY, &muted) == ESP_OK) {
        s_settings.mic_muted = muted != 0;
    }
    uint8_t volume = 0;
    if (nvs_get_u8(nvs, VOLUME_KEY, &volume) == ESP_OK && volume <= 100) {
        s_settings.volume = volume;
    }
    uint8_t eyes = 0;
    if (nvs_get_u8(nvs, EYES_KEY, &eyes) == ESP_OK) {
        s_settings.eyes = eyes != 0;
    }
    uint8_t idle_eyes = 0;
    if (nvs_get_u8(nvs, IDLE_EYES_KEY, &idle_eyes) == ESP_OK) {
        s_settings.idle_eyes = idle_eyes != 0;
    }
    uint8_t wake_sound = 0;
    if (nvs_get_u8(nvs, WAKE_SOUND_KEY, &wake_sound) == ESP_OK) {
        s_settings.wake_sound = wake_sound != 0;
    }
    uint8_t pet_sounds = 0;
    if (nvs_get_u8(nvs, PET_SOUNDS_KEY, &pet_sounds) == ESP_OK) {
        s_settings.pet_sounds = pet_sounds != 0;
    }
    uint8_t pet_react = 0;
    if (nvs_get_u8(nvs, PET_REACT_KEY, &pet_react) == ESP_OK) {
        s_settings.pet_reactions = pet_react != 0;
    }
    size_t len = sizeof(s_settings.wake_word);
    if (nvs_get_str(nvs, WAKE_WORD_KEY, s_settings.wake_word, &len) != ESP_OK) {
        s_settings.wake_word[0] = '\0';
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
    s_settings.source[UI_SCREEN_CPU] = METRICS_SOURCE_CPU;
    s_settings.source[UI_SCREEN_GPU] = METRICS_SOURCE_GPU;
    s_settings.volume = BOARD_VOLUME_DEFAULT;
    s_settings.eyes = true;
    s_settings.idle_eyes = true;
    s_settings.wake_sound = true;
    s_settings.pet_sounds = true;
    s_settings.pet_reactions = true;
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

esp_err_t board_settings_set_source(int screen, metrics_source_t source)
{
    if (!valid_screen(screen) || source >= METRICS_SOURCE_COUNT) {
        return ESP_ERR_INVALID_ARG;
    }
    xSemaphoreTake(s_lock, portMAX_DELAY);
    bool changed = s_settings.source[screen] != source;
    s_settings.source[screen] = source;
    xSemaphoreGive(s_lock);
    if (changed) {
        save(SRC_KEYS[screen], source, false);
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

esp_err_t board_settings_set_mic_muted(bool muted)
{
    xSemaphoreTake(s_lock, portMAX_DELAY);
    bool changed = s_settings.mic_muted != muted;
    s_settings.mic_muted = muted;
    xSemaphoreGive(s_lock);
    if (changed) {
        save(MIC_MUTED_KEY, muted, false);
    }
    return ESP_OK;
}

esp_err_t board_settings_set_volume(uint8_t percent)
{
    if (percent > 100) {
        return ESP_ERR_INVALID_ARG;
    }
    esp_err_t err = board_audio_set_volume(percent);
    if (err != ESP_OK) {
        return err;
    }
    xSemaphoreTake(s_lock, portMAX_DELAY);
    bool changed = s_settings.volume != percent;
    s_settings.volume = percent;
    xSemaphoreGive(s_lock);
    if (changed) {
        save(VOLUME_KEY, percent, false);
    }
    return ESP_OK;
}

esp_err_t board_settings_set_eyes(bool on)
{
    xSemaphoreTake(s_lock, portMAX_DELAY);
    bool changed = s_settings.eyes != on;
    s_settings.eyes = on;
    xSemaphoreGive(s_lock);
    if (changed) {
        save(EYES_KEY, on, false);
    }
    return ESP_OK;
}

esp_err_t board_settings_set_idle_eyes(bool on)
{
    xSemaphoreTake(s_lock, portMAX_DELAY);
    bool changed = s_settings.idle_eyes != on;
    s_settings.idle_eyes = on;
    xSemaphoreGive(s_lock);
    if (changed) {
        save(IDLE_EYES_KEY, on, false);
    }
    return ESP_OK;
}

esp_err_t board_settings_set_wake_sound(bool on)
{
    xSemaphoreTake(s_lock, portMAX_DELAY);
    bool changed = s_settings.wake_sound != on;
    s_settings.wake_sound = on;
    xSemaphoreGive(s_lock);
    if (changed) {
        save(WAKE_SOUND_KEY, on, false);
    }
    return ESP_OK;
}

esp_err_t board_settings_set_pet_sounds(bool on)
{
    xSemaphoreTake(s_lock, portMAX_DELAY);
    bool changed = s_settings.pet_sounds != on;
    s_settings.pet_sounds = on;
    xSemaphoreGive(s_lock);
    if (changed) {
        save(PET_SOUNDS_KEY, on, false);
    }
    return ESP_OK;
}

esp_err_t board_settings_set_pet_reactions(bool on)
{
    xSemaphoreTake(s_lock, portMAX_DELAY);
    bool changed = s_settings.pet_reactions != on;
    s_settings.pet_reactions = on;
    xSemaphoreGive(s_lock);
    if (changed) {
        save(PET_REACT_KEY, on, false);
    }
    return ESP_OK;
}

esp_err_t board_settings_set_wake_word(const char *id)
{
    if (strlen(id) >= sizeof(s_settings.wake_word)) {
        return ESP_ERR_INVALID_ARG;
    }
    xSemaphoreTake(s_lock, portMAX_DELAY);
    bool changed = strcmp(s_settings.wake_word, id) != 0;
    strlcpy(s_settings.wake_word, id, sizeof(s_settings.wake_word));
    xSemaphoreGive(s_lock);
    if (!changed) {
        return ESP_OK;
    }
    nvs_handle_t nvs;
    esp_err_t err = nvs_open(NVS_NAMESPACE, NVS_READWRITE, &nvs);
    if (err == ESP_OK) {
        err = nvs_set_str(nvs, WAKE_WORD_KEY, id);
        if (err == ESP_OK) {
            err = nvs_commit(nvs);
        }
        nvs_close(nvs);
    }
    if (err != ESP_OK) {
        ESP_LOGW(TAG, "saving %s failed: %s", WAKE_WORD_KEY, esp_err_to_name(err));
    }
    return ESP_OK;
}
