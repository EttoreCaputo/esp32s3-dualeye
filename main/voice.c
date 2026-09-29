#include "voice.h"

#include <string.h>

#include "board_audio.h"
#include "cJSON.h"
#include "esp_afe_config.h"
#include "esp_afe_sr_models.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "esp_wn_models.h"
#include "freertos/FreeRTOS.h"
#include "freertos/event_groups.h"
#include "freertos/semphr.h"
#include "freertos/task.h"
#include "lvgl_port.h"
#include "model_path.h"
#include "rpc.h"
#include "ui_voice.h"

static const char *TAG = "voice";

/* ES7210 TDM slots as M0 found them: mic, speaker loopback (AEC reference),
 * two unconnected. The AFE reads the interleaved frames as they come. */
#define VOICE_INPUT_FORMAT "MRNN"
#define VOICE_MODEL_PARTITION "model"
/* Until M4 streams the utterance and the host says when it's done. */
#define LISTEN_TIMEOUT_MS 6000
/* A host that set thinking or speaking and went away doesn't leave it on. */
#define HOST_STATE_TIMEOUT_MS 30000
#define FETCH_WAIT_MS 100
#define TASK_CORE 1
#define TASK_PRIORITY 5

#define RUN_BIT BIT0

static const char *const STATE_NAMES[VOICE_STATE_COUNT] = {
    [VOICE_IDLE] = "idle",
    [VOICE_LISTENING] = "listening",
    [VOICE_THINKING] = "thinking",
    [VOICE_SPEAKING] = "speaking",
};

static const esp_afe_sr_iface_t *s_afe;
static esp_afe_sr_data_t *s_afe_data;
static char s_model[32];
static bool s_available;

/* The feed task holds it for each chunk it reads, a pause for as long as it lasts. */
static SemaphoreHandle_t s_mic;
/* RUN_BIT: the feed task may read the mic (not muted, not paused). */
static EventGroupHandle_t s_run;
static SemaphoreHandle_t s_ctl;
static bool s_muted;
static bool s_paused;

static SemaphoreHandle_t s_state_lock;
static voice_state_t s_state;
static int64_t s_state_until_us;

const char *voice_state_name(voice_state_t state)
{
    return state < VOICE_STATE_COUNT ? STATE_NAMES[state] : "idle";
}

bool voice_state_from_name(const char *name, voice_state_t *out)
{
    for (int i = 0; i < VOICE_STATE_COUNT; i++) {
        if (strcmp(name, STATE_NAMES[i]) == 0) {
            *out = (voice_state_t) i;
            return true;
        }
    }
    return false;
}

bool voice_available(void)
{
    return s_available;
}

const char *voice_wake_model(void)
{
    return s_available ? s_model : NULL;
}

const char *voice_wake_word(void)
{
    static const struct {
        const char *key;
        const char *word;
    } WORDS[] = {
        {"hiesp", "Hi ESP"}, {"alexa", "Alexa"}, {"jarvis", "Jarvis"}, {"computer", "Computer"},
    };
    if (!s_available) {
        return NULL;
    }
    for (size_t i = 0; i < sizeof(WORDS) / sizeof(WORDS[0]); i++) {
        if (strstr(s_model, WORDS[i].key) != NULL) {
            return WORDS[i].word;
        }
    }
    return s_model;
}

static void update_run(void)
{
    if (!s_muted && !s_paused) {
        xEventGroupSetBits(s_run, RUN_BIT);
    } else {
        xEventGroupClearBits(s_run, RUN_BIT);
    }
}

void voice_set_muted(bool muted)
{
    if (!s_available) {
        s_muted = muted;
        return;
    }
    xSemaphoreTake(s_ctl, portMAX_DELAY);
    s_muted = muted;
    update_run();
    xSemaphoreGive(s_ctl);
    ESP_LOGI(TAG, "wake word %s", muted ? "muted" : "listening");
    if (muted) {
        voice_set_state(VOICE_IDLE);
    }
}

bool voice_muted(void)
{
    return s_muted;
}

void voice_pause(bool pause)
{
    if (!s_available || pause == s_paused) {
        return;
    }
    xSemaphoreTake(s_ctl, portMAX_DELAY);
    s_paused = pause;
    update_run();
    xSemaphoreGive(s_ctl);
    // The feed task checks RUN_BIT before each chunk; taking the mic waits for
    // the one it may be reading.
    if (pause) {
        xSemaphoreTake(s_mic, portMAX_DELAY);
    } else {
        xSemaphoreGive(s_mic);
    }
}

voice_state_t voice_state(void)
{
    return s_state;
}

static void notify_state(voice_state_t state)
{
    cJSON *params = cJSON_CreateObject();
    cJSON_AddStringToObject(params, "state", voice_state_name(state));
    rpc_notify("voice_state", params);
}

void voice_set_state(voice_state_t state)
{
    if (s_state_lock == NULL || state >= VOICE_STATE_COUNT) {
        return;
    }
    xSemaphoreTake(s_state_lock, portMAX_DELAY);
    bool changed = s_state != state;
    s_state = state;
    int timeout_ms = state == VOICE_LISTENING ? LISTEN_TIMEOUT_MS : HOST_STATE_TIMEOUT_MS;
    s_state_until_us = esp_timer_get_time() + (int64_t) timeout_ms * 1000;
    if (changed) {
        lvgl_port_lock();
        ui_voice_show(state);
        lvgl_port_unlock();
    }
    xSemaphoreGive(s_state_lock);
    if (changed) {
        notify_state(state);
    }
}

static void on_wake(const afe_fetch_result_t *res)
{
    ESP_LOGI(TAG, "wake word \"%s\" (%.1f dBFS)", voice_wake_word(), res->data_volume);
    cJSON *params = cJSON_CreateObject();
    cJSON_AddStringToObject(params, "word", voice_wake_word());
    cJSON_AddStringToObject(params, "model", s_model);
    cJSON_AddNumberToObject(params, "volume_db", (int) res->data_volume);
    rpc_notify("wake", params);
    voice_set_state(VOICE_LISTENING);
}

static void feed_task(void *arg)
{
    const int chunk = s_afe->get_feed_chunksize(s_afe_data);
    int16_t *buf = heap_caps_malloc((size_t) chunk * BOARD_AUDIO_IN_CHANNELS * sizeof(int16_t),
                                    MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);
    if (buf == NULL) {
        ESP_LOGE(TAG, "no memory for the feed buffer");
        vTaskDelete(NULL);
    }
    while (true) {
        xEventGroupWaitBits(s_run, RUN_BIT, pdFALSE, pdTRUE, portMAX_DELAY);
        xSemaphoreTake(s_mic, portMAX_DELAY);
        esp_err_t err = board_audio_read(buf, (size_t) chunk);
        xSemaphoreGive(s_mic);
        if (err == ESP_OK) {
            s_afe->feed(s_afe_data, buf);
        } else {
            vTaskDelay(pdMS_TO_TICKS(10));
        }
    }
}

static void fetch_task(void *arg)
{
    while (true) {
        // Muted or paused nothing is fed, and fetching would only get the AFE
        // to warn about its empty buffer every time.
        EventBits_t run = xEventGroupWaitBits(s_run, RUN_BIT, pdFALSE, pdTRUE, pdMS_TO_TICKS(FETCH_WAIT_MS));
        afe_fetch_result_t *res = NULL;
        if (run & RUN_BIT) {
            res = s_afe->fetch_with_delay(s_afe_data, pdMS_TO_TICKS(FETCH_WAIT_MS));
        }
        if (res != NULL && res->ret_value != ESP_FAIL && res->wakeup_state == WAKENET_DETECTED) {
            on_wake(res);
        }
        if (s_state != VOICE_IDLE && esp_timer_get_time() >= s_state_until_us) {
            voice_set_state(VOICE_IDLE);
        }
    }
}

void voice_start(bool muted)
{
    s_muted = muted;
    s_state_lock = xSemaphoreCreateMutex();

    srmodel_list_t *models = esp_srmodel_init(VOICE_MODEL_PARTITION);
    if (models == NULL || models->num == 0) {
        ESP_LOGE(TAG, "no ESP-SR models in the \"%s\" partition, continuing without voice", VOICE_MODEL_PARTITION);
        return;
    }
    afe_config_t *cfg = afe_config_init(VOICE_INPUT_FORMAT, models, AFE_TYPE_SR, AFE_MODE_LOW_COST);
    if (cfg == NULL || cfg->wakenet_model_name == NULL) {
        ESP_LOGE(TAG, "no WakeNet model, continuing without voice");
        afe_config_free(cfg);
        return;
    }
    // Internal RAM is short (M0: 36 KB largest block); PSRAM has 8 MB.
    cfg->memory_alloc_mode = AFE_MEMORY_ALLOC_MORE_PSRAM;
    cfg->afe_perferred_core = TASK_CORE;
    cfg->afe_perferred_priority = TASK_PRIORITY;
    strlcpy(s_model, cfg->wakenet_model_name, sizeof(s_model));
    s_afe = esp_afe_handle_from_config(cfg);
    s_afe_data = s_afe != NULL ? s_afe->create_from_config(cfg) : NULL;
    afe_config_free(cfg);
    if (s_afe_data == NULL) {
        ESP_LOGE(TAG, "AFE create failed, continuing without voice");
        return;
    }
    if (s_afe->get_feed_channel_num(s_afe_data) != BOARD_AUDIO_IN_CHANNELS) {
        ESP_LOGE(TAG, "AFE wants %d channels, the ES7210 gives %d", s_afe->get_feed_channel_num(s_afe_data),
                 BOARD_AUDIO_IN_CHANNELS);
        return;
    }

    s_mic = xSemaphoreCreateMutex();
    s_run = xEventGroupCreate();
    s_ctl = xSemaphoreCreateMutex();
    if (s_mic == NULL || s_run == NULL || s_ctl == NULL) {
        ESP_LOGE(TAG, "out of memory, continuing without voice");
        return;
    }
    update_run();
    // Core 1, clear of LVGL on core 0. Fetch runs WakeNet, hence its stack.
    if (xTaskCreatePinnedToCore(feed_task, "voice_feed", 4096, NULL, TASK_PRIORITY, NULL, TASK_CORE) != pdPASS
        || xTaskCreatePinnedToCore(fetch_task, "voice_fetch", 8192, NULL, TASK_PRIORITY, NULL, TASK_CORE) != pdPASS) {
        ESP_LOGE(TAG, "voice tasks not started");
        return;
    }
    s_available = true;
    ESP_LOGI(TAG, "wake word \"%s\" (%s), feed %d samples x %d ch%s", voice_wake_word(), s_model,
             s_afe->get_feed_chunksize(s_afe_data), BOARD_AUDIO_IN_CHANNELS, muted ? ", muted" : "");
}
