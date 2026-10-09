#include <string.h>

#include "art.h"
#include "audio_selftest.h"
#include "board_display.h"
#include "board_settings.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "link.h"
#include "lvgl_port.h"
#include "media.h"
#include "metrics_io.h"
#include "metrics_model.h"
#include "rpc.h"
#include "board_audio.h"
#include "pet.h"
#include "playback.h"
#include "ui_toast.h"
#include "ui_eyes.h"
#include "ui_voice.h"
#include "ui_watch.h"
#include "voice.h"

static const char *TAG = "dualeye";

static board_lcd_t s_lcds[BOARD_LCD_COUNT];
static lv_display_t *s_displays[BOARD_LCD_COUNT];
/* Rotation each screen is drawn with right now. */
static uint16_t s_rot[BOARD_LCD_COUNT];

/** Call with the LVGL lock held: no frame is being drawn meanwhile. */
static void apply_rotation(const uint16_t want[BOARD_LCD_COUNT])
{
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        if (want[i] == s_rot[i]) {
            continue;
        }
        if (board_display_rotate(s_lcds[i].panel, i, (board_lcd_rotation_t) want[i]) != ESP_OK) {
            continue;
        }
        s_rot[i] = want[i];
        // Everything on the panel was drawn the old way round.
        lv_obj_invalidate(lv_display_get_screen_active(s_displays[i]));
        ESP_LOGI(TAG, "screen %d turned %u deg", i, want[i]);
    }
}

/** The metrics with the faces each screen is set to. */
static void current_view(metrics_snapshot_t *snap, board_settings_t *settings)
{
    metrics_model_get(snap);
    board_settings_get(settings);
    snap->cpu_face = settings->face[UI_SCREEN_CPU];
    snap->gpu_face = settings->face[UI_SCREEN_GPU];
    snap->cpu_source = settings->source[UI_SCREEN_CPU];
    snap->gpu_source = settings->source[UI_SCREEN_GPU];
}

/* While the host says a timer rings: the chime every ALARM_EVERY_MS, unless
 * someone is talking to the board (the host stops it on the wake word), for
 * ALARM_MAX_MS at most even if the host doesn't. Most of each period is
 * silence, for the wake word to be heard in. */
#define ALARM_EVERY_MS 2000
#define ALARM_MAX_MS 10000

static void ring_alarm(const metrics_snapshot_t *snap, uint32_t *since_ms, uint32_t *last_ms)
{
    uint32_t now = (uint32_t) (esp_timer_get_time() / 1000);
    bool ringing = snap->timer.valid && snap->timer.ringing && snap->state == METRICS_UI_LIVE;
    if (!ringing) {
        *since_ms = 0;
        *last_ms = 0;
        return;
    }
    if (*since_ms == 0) {
        *since_ms = now;
    }
    if (now - *since_ms >= ALARM_MAX_MS || voice_state() != VOICE_IDLE || playback_active()) {
        return;
    }
    if (*last_ms != 0 && now - *last_ms < ALARM_EVERY_MS) {
        return;
    }
    *last_ms = now;
    playback_sound(SOUND_ALARM);
}

static void ui_refresh_task(void *arg)
{
    metrics_snapshot_t prev;
    memset(&prev, 0, sizeof(prev));
    uint32_t alarm_since_ms = 0;
    uint32_t alarm_ms = 0;

    while (true) {
        metrics_snapshot_t snap;
        board_settings_t settings;
        current_view(&snap, &settings);
        ring_alarm(&snap, &alarm_since_ms, &alarm_ms);
        pet_update(&snap);
        if (snap.state == METRICS_UI_LIVE && prev.state != METRICS_UI_LIVE) {
            // The host is here (again): say hi.
            playback_sound(SOUND_HELLO);
        }
        bool turned = memcmp(settings.rot, s_rot, sizeof(s_rot)) != 0;
        if (turned || memcmp(&prev, &snap, sizeof(snap)) != 0) {
            lvgl_port_lock();
            apply_rotation(settings.rot);
            ui_watch_update(&snap);
            lvgl_port_unlock();
            prev = snap;
        }
        vTaskDelay(pdMS_TO_TICKS(100));
    }
}

void app_main(void)
{
    // First, so that everything logged from here on reaches the host in frames.
    ESP_ERROR_CHECK(link_init());
    board_settings_init();
    media_init();
    art_init();
    board_settings_t settings;
    board_settings_get(&settings);
    pet_init();
    pet_set_reactions(settings.pet_reactions);

    ESP_ERROR_CHECK(board_display_init(s_lcds));
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        ESP_ERROR_CHECK(board_display_rotate(s_lcds[i].panel, i, (board_lcd_rotation_t) settings.rot[i]));
        s_rot[i] = settings.rot[i];
    }

    metrics_model_init();

    ESP_ERROR_CHECK(lvgl_port_init(s_lcds, s_displays));

    lvgl_port_lock();
    ui_watch_create(s_displays[UI_SCREEN_CPU], s_displays[UI_SCREEN_GPU]);
    ui_voice_create(s_displays);
    ui_voice_set_eyes(settings.eyes);
    ui_eyes_set_idle(settings.idle_eyes);
    voice_set_wake_sound(settings.wake_sound);
    ui_toast_create(s_displays);

    metrics_snapshot_t snap;
    current_view(&snap, &settings);
    ui_watch_update(&snap);
    lvgl_port_unlock();

    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        board_display_set_brightness(i, settings.brightness[i]);
    }

    // UI on core 0, audio (self-test, wake word) on core 1.
    BaseType_t ui_ok = xTaskCreatePinnedToCore(ui_refresh_task, "ui_refresh", 4096, NULL, 4, NULL, 0);
    ESP_ERROR_CHECK(ui_ok == pdPASS ? ESP_OK : ESP_ERR_NO_MEM);
    if (board_audio_init() == ESP_OK) {
        board_audio_set_volume(settings.volume);
        audio_selftest_start();
        voice_start(settings.mic_muted, settings.wake_word);
        playback_start();
        playback_set_pet_sounds(settings.pet_sounds);
        playback_sound(SOUND_BOOT);
    } else {
        ESP_LOGE(TAG, "audio init failed, continuing without audio");
    }

    rpc_init();
    link_on_receive(LINK_CHAN_CTRL, rpc_handle);
    link_on_receive(LINK_CHAN_METRICS, metrics_io_handle);
    link_on_receive(LINK_CHAN_AUDIO_DOWN, playback_receive);
    link_start();
    // A host already listening (e.g. right after flashing) learns the board rebooted.
    rpc_announce();
    ESP_LOGI(TAG, "Watch UI ready, waiting for the host");
}
