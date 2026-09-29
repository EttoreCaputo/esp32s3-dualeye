#include "voice.h"

#include <string.h>

#include "board_audio.h"
#include "link.h"
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
/* The utterance after the wake word, streamed on `audio_up`: it ends after
 * VAD_HANGOVER_MS + END_SILENCE_MS of silence once speech started,
 * NO_SPEECH_MS without any, or MAX_UTTERANCE_MS in all. */
#define VAD_HANGOVER_MS 500
#define END_SILENCE_MS 250
/* The VAD still reports the wake word itself for its hangover: speech that
 * doesn't outlast it isn't the command. */
#define WAKE_TAIL_MS (VAD_HANGOVER_MS + 200)
#define NO_SPEECH_MS 5000
#define MAX_UTTERANCE_MS 12000
/* Frame header: utterance id, flags (none yet), sequence number (u16 LE). */
#define AUDIO_HEADER 4
/* The fetch task mustn't stall on a host that doesn't read. */
#define AUDIO_SEND_WAIT_MS 20
/* Level shown on the ring: this range of dBFS, updated this often. */
#define LEVEL_FLOOR_DB -65.0f
#define LEVEL_CEIL_DB -30.0f
#define LEVEL_EVERY_MS 100
#define LISTEN_TIMEOUT_MS (MAX_UTTERANCE_MS + 1000)
/* A host that set thinking or speaking and went away doesn't leave it on. */
#define HOST_STATE_TIMEOUT_MS 30000
#define FETCH_WAIT_MS 100
#define TASK_CORE 1
#define TASK_PRIORITY 5

#define RUN_BIT BIT0
/* Host requests, for the fetch task, which owns the stream. */
#define LISTEN_BIT BIT1
#define STOP_BIT BIT2

typedef enum {
    END_SPEECH,
    END_NO_SPEECH,
    END_MAX_LENGTH,
    END_HOST,
    END_MUTED,
} end_reason_t;

static const char *const END_NAMES[] = {
    [END_SPEECH] = "end_of_speech", [END_NO_SPEECH] = "no_speech", [END_MAX_LENGTH] = "max_length",
    [END_HOST] = "host", [END_MUTED] = "muted",
};

/* The utterance being streamed. Only the fetch task touches it. */
typedef struct {
    bool active;
    uint8_t id;
    uint16_t seq;
    int64_t start_us;
    int64_t ignore_until_us; /* VAD speech before this is the wake word's */
    int64_t speech_us; /* last frame with speech; 0 before any */
    uint32_t samples;
    uint32_t dropped;
    int64_t level_us;
    uint8_t *frame;
} stream_t;

static stream_t s_stream;

static const char *const STATE_NAMES[VOICE_STATE_COUNT] = {
    [VOICE_IDLE] = "idle",
    [VOICE_LISTENING] = "listening",
    [VOICE_THINKING] = "thinking",
    [VOICE_SPEAKING] = "speaking",
};

/* Wake words people can pick, by the WakeNet model names that carry them.
 * Those in the `model` partition are set in sdkconfig.defaults. */
static const struct {
    const char *id;
    const char *word;
} WORDS[] = {
    {"alexa", "Alexa"}, {"hiesp", "Hi ESP"}, {"jarvis", "Jarvis"}, {"computer", "Computer"},
};
#define WORD_COUNT (sizeof(WORDS) / sizeof(WORDS[0]))
#define DEFAULT_WORD "alexa"

static srmodel_list_t *s_models;
static const esp_afe_sr_iface_t *s_afe;
static esp_afe_sr_data_t *s_afe_data;
static int s_chunk;
static char s_model[32];
static bool s_available;

/* The feed task holds it for each chunk it reads and feeds, a pause for as
 * long as it lasts. */
static SemaphoreHandle_t s_mic;
/* The fetch task holds it while it waits on the AFE. With both, the AFE can
 * be swapped for one with another wake word. */
static SemaphoreHandle_t s_fetch;
/* RUN_BIT: the feed task may read the mic (not muted, not paused). */
static EventGroupHandle_t s_run;
static SemaphoreHandle_t s_ctl;
static EventGroupHandle_t s_req;
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

static int word_of_model(const char *model)
{
    for (int i = 0; i < (int) WORD_COUNT; i++) {
        if (strstr(model, WORDS[i].id) != NULL) {
            return i;
        }
    }
    return -1;
}

const char *voice_wake_word(void)
{
    if (!s_available) {
        return NULL;
    }
    int w = word_of_model(s_model);
    return w >= 0 ? WORDS[w].word : s_model;
}

const char *voice_wake_word_id(void)
{
    if (!s_available) {
        return NULL;
    }
    int w = word_of_model(s_model);
    return w >= 0 ? WORDS[w].id : s_model;
}

/** The WakeNet model for a word id in the partition, or NULL. */
static char *model_for(const char *id)
{
    return id != NULL && id[0] != '\0' ? esp_srmodel_filter(s_models, ESP_WN_PREFIX, id) : NULL;
}

int voice_wake_words(const char **ids, int max)
{
    int n = 0;
    for (int i = 0; i < (int) WORD_COUNT && n < max && s_models != NULL; i++) {
        if (model_for(WORDS[i].id) != NULL) {
            ids[n++] = WORDS[i].id;
        }
    }
    return n;
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

static void stream_start(const char *trigger, bool after_wake)
{
    stream_t *st = &s_stream;
    st->active = true;
    st->id++;
    st->seq = 0;
    st->start_us = esp_timer_get_time();
    st->ignore_until_us = after_wake ? st->start_us + WAKE_TAIL_MS * 1000 : 0;
    st->speech_us = 0;
    st->samples = 0;
    st->dropped = 0;
    st->level_us = 0;
    // "Alexa" said again mid-sentence isn't a new wake, and WakeNet's CPU is
    // free meanwhile.
    s_afe->disable_wakenet(s_afe_data);
    cJSON *params = cJSON_CreateObject();
    cJSON_AddNumberToObject(params, "id", st->id);
    cJSON_AddStringToObject(params, "trigger", trigger);
    cJSON_AddNumberToObject(params, "rate", 16000);
    cJSON_AddStringToObject(params, "format", "s16le");
    rpc_notify("utterance_start", params);
    voice_set_state(VOICE_LISTENING);
}

static void stream_end(end_reason_t reason)
{
    stream_t *st = &s_stream;
    if (!st->active) {
        return;
    }
    st->active = false;
    s_afe->enable_wakenet(s_afe_data);
    int ms = (int) (st->samples / 16);
    ESP_LOGI(TAG, "utterance %d: %d ms, %s%s", st->id, ms, END_NAMES[reason],
             st->dropped ? " (frames dropped)" : "");
    cJSON *params = cJSON_CreateObject();
    cJSON_AddNumberToObject(params, "id", st->id);
    cJSON_AddStringToObject(params, "reason", END_NAMES[reason]);
    cJSON_AddNumberToObject(params, "ms", ms);
    cJSON_AddBoolToObject(params, "speech", st->speech_us != 0);
    cJSON_AddNumberToObject(params, "frames", st->seq);
    cJSON_AddNumberToObject(params, "dropped", st->dropped);
    rpc_notify("utterance_end", params);
    // With speech the host transcribes it, and says what comes next.
    voice_set_state(st->speech_us != 0 && reason != END_MUTED ? VOICE_THINKING : VOICE_IDLE);
    lvgl_port_lock();
    ui_voice_set_level(0);
    lvgl_port_unlock();
}

static void stream_frame(const afe_fetch_result_t *res)
{
    stream_t *st = &s_stream;
    int64_t now = esp_timer_get_time();
    size_t bytes = (size_t) res->data_size;
    if (bytes > LINK_MAX_PAYLOAD - AUDIO_HEADER) {
        bytes = LINK_MAX_PAYLOAD - AUDIO_HEADER;
    }
    st->frame[0] = st->id;
    st->frame[1] = 0;
    st->frame[2] = (uint8_t) (st->seq & 0xff);
    st->frame[3] = (uint8_t) (st->seq >> 8);
    memcpy(st->frame + AUDIO_HEADER, res->data, bytes);
    if (link_send_timeout(LINK_CHAN_AUDIO_UP, st->frame, AUDIO_HEADER + bytes, AUDIO_SEND_WAIT_MS) != ESP_OK) {
        st->dropped++;
    }
    st->seq++;
    st->samples += bytes / sizeof(int16_t);
    if (res->vad_state == VAD_SPEECH && now >= st->ignore_until_us) {
        st->speech_us = now;
    }

    if (now - st->level_us >= LEVEL_EVERY_MS * 1000) {
        st->level_us = now;
        float level = (res->data_volume - LEVEL_FLOOR_DB) / (LEVEL_CEIL_DB - LEVEL_FLOOR_DB);
        lvgl_port_lock();
        ui_voice_set_level(level < 0 ? 0 : level > 1 ? 1 : level);
        lvgl_port_unlock();
    }

    int64_t elapsed_ms = (now - st->start_us) / 1000;
    if (st->speech_us != 0 && (now - st->speech_us) / 1000 >= END_SILENCE_MS) {
        stream_end(END_SPEECH);
    } else if (st->speech_us == 0 && elapsed_ms >= NO_SPEECH_MS) {
        stream_end(END_NO_SPEECH);
    } else if (elapsed_ms >= MAX_UTTERANCE_MS) {
        stream_end(END_MAX_LENGTH);
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
    stream_start("wake", true);
}

static void feed_task(void *arg)
{
    // Every AFE voice_set_wake_word() builds has this chunk size.
    const int chunk = s_chunk;
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
        if (err == ESP_OK) {
            s_afe->feed(s_afe_data, buf);
        }
        xSemaphoreGive(s_mic);
        if (err != ESP_OK) {
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
        EventBits_t req = xEventGroupClearBits(s_req, LISTEN_BIT | STOP_BIT);
        // s_stream is only touched with s_fetch held.
        xSemaphoreTake(s_fetch, portMAX_DELAY);
        if (!(run & RUN_BIT)) {
            stream_end(END_MUTED);
        } else {
            if (req & STOP_BIT) {
                stream_end(END_HOST);
            }
            if ((req & LISTEN_BIT) && !s_stream.active) {
                stream_start("host", false);
            }
            afe_fetch_result_t *res = s_afe->fetch_with_delay(s_afe_data, pdMS_TO_TICKS(FETCH_WAIT_MS));
            if (res != NULL && res->ret_value != ESP_FAIL) {
                if (s_stream.active) {
                    stream_frame(res);
                } else if (res->wakeup_state == WAKENET_DETECTED) {
                    on_wake(res);
                }
            }
        }
        bool streaming = s_stream.active;
        xSemaphoreGive(s_fetch);
        if (!streaming && s_state != VOICE_IDLE && esp_timer_get_time() >= s_state_until_us) {
            voice_set_state(VOICE_IDLE);
        }
    }
}

bool voice_listen(void)
{
    if (!s_available || s_muted) {
        return false;
    }
    xEventGroupSetBits(s_req, LISTEN_BIT);
    return true;
}

void voice_stop_listening(void)
{
    if (s_available) {
        xEventGroupSetBits(s_req, STOP_BIT);
    }
}

/** Build the AFE with the WakeNet `model`. */
static esp_afe_sr_data_t *create_afe(const char *model)
{
    afe_config_t *cfg = afe_config_init(VOICE_INPUT_FORMAT, s_models, AFE_TYPE_SR, AFE_MODE_LOW_COST);
    if (cfg == NULL) {
        return NULL;
    }
    // afe_config_init() runs the first two WakeNets in the partition; one is
    // enough, and half the CPU. Its strings go back before afe_config_free(),
    // which owns them.
    char *first = cfg->wakenet_model_name;
    char *second = cfg->wakenet_model_name_2;
    cfg->wakenet_model_name = (char *) model;
    cfg->wakenet_model_name_2 = NULL;
    // Silence ends an utterance sooner than the default 1000 ms.
    cfg->vad_min_noise_ms = VAD_HANGOVER_MS;
    // Internal RAM is short (M0: 36 KB largest block); PSRAM has 8 MB.
    cfg->memory_alloc_mode = AFE_MEMORY_ALLOC_MORE_PSRAM;
    cfg->afe_perferred_core = TASK_CORE;
    cfg->afe_perferred_priority = TASK_PRIORITY;
    s_afe = esp_afe_handle_from_config(cfg);
    esp_afe_sr_data_t *data = s_afe != NULL ? s_afe->create_from_config(cfg) : NULL;
    cfg->wakenet_model_name = first;
    cfg->wakenet_model_name_2 = second;
    afe_config_free(cfg);
    return data;
}

esp_err_t voice_set_wake_word(const char *id)
{
    if (!s_available) {
        return ESP_ERR_INVALID_STATE;
    }
    char *model = model_for(id);
    if (model == NULL) {
        return ESP_ERR_NOT_FOUND;
    }
    if (strcmp(model, s_model) == 0) {
        return ESP_OK;
    }
    // Neither task is in the AFE while we hold both. A self-test `rec` holds
    // the mic, so this waits for it.
    xSemaphoreTake(s_mic, portMAX_DELAY);
    xSemaphoreTake(s_fetch, portMAX_DELAY);
    stream_end(END_HOST);
    s_afe->destroy(s_afe_data);
    esp_err_t err = ESP_OK;
    s_afe_data = create_afe(model);
    if (s_afe_data == NULL || s_afe->get_feed_chunksize(s_afe_data) != s_chunk) {
        ESP_LOGE(TAG, "AFE with %s failed, back to %s", model, s_model);
        if (s_afe_data != NULL) {
            s_afe->destroy(s_afe_data);
        }
        s_afe_data = create_afe(s_model);
        err = ESP_FAIL;
    } else {
        strlcpy(s_model, model, sizeof(s_model));
    }
    xSemaphoreGive(s_fetch);
    xSemaphoreGive(s_mic);
    if (s_afe_data == NULL) {
        // Nothing to feed or fetch from: stop both tasks for good. Without
        // s_available, nothing sets RUN_BIT again.
        ESP_LOGE(TAG, "AFE lost, voice off until a reboot");
        xSemaphoreTake(s_ctl, portMAX_DELAY);
        s_available = false;
        s_paused = true;
        update_run();
        xSemaphoreGive(s_ctl);
        voice_set_state(VOICE_IDLE);
        return ESP_FAIL;
    }
    voice_set_state(VOICE_IDLE);
    ESP_LOGI(TAG, "wake word \"%s\" (%s)", voice_wake_word(), s_model);
    return err;
}

void voice_start(bool muted, const char *wake_word)
{
    s_muted = muted;
    s_state_lock = xSemaphoreCreateMutex();

    s_models = esp_srmodel_init(VOICE_MODEL_PARTITION);
    if (s_models == NULL || s_models->num == 0) {
        ESP_LOGE(TAG, "no ESP-SR models in the \"%s\" partition, continuing without voice", VOICE_MODEL_PARTITION);
        return;
    }
    char *model = model_for(wake_word);
    if (model == NULL) {
        model = model_for(DEFAULT_WORD);
    }
    if (model == NULL) {
        model = esp_srmodel_filter(s_models, ESP_WN_PREFIX, NULL);
    }
    if (model == NULL) {
        ESP_LOGE(TAG, "no WakeNet model, continuing without voice");
        return;
    }
    strlcpy(s_model, model, sizeof(s_model));
    s_afe_data = create_afe(model);
    if (s_afe_data == NULL) {
        ESP_LOGE(TAG, "AFE create failed, continuing without voice");
        return;
    }
    if (s_afe->get_feed_channel_num(s_afe_data) != BOARD_AUDIO_IN_CHANNELS) {
        ESP_LOGE(TAG, "AFE wants %d channels, the ES7210 gives %d", s_afe->get_feed_channel_num(s_afe_data),
                 BOARD_AUDIO_IN_CHANNELS);
        return;
    }
    s_chunk = s_afe->get_feed_chunksize(s_afe_data);

    s_mic = xSemaphoreCreateMutex();
    s_fetch = xSemaphoreCreateMutex();
    s_run = xEventGroupCreate();
    s_ctl = xSemaphoreCreateMutex();
    s_req = xEventGroupCreate();
    s_stream.frame = heap_caps_malloc(LINK_MAX_PAYLOAD, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    if (s_mic == NULL || s_fetch == NULL || s_run == NULL || s_ctl == NULL || s_req == NULL
        || s_stream.frame == NULL) {
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
    ESP_LOGI(TAG, "wake word \"%s\" (%s), feed %d samples x %d ch%s", voice_wake_word(), s_model, s_chunk,
             BOARD_AUDIO_IN_CHANNELS, muted ? ", muted" : "");
}
