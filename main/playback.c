#include "playback.h"

#include <math.h>
#include <string.h>

#include "board_audio.h"
#include "cJSON.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "freertos/task.h"
#include "lvgl_port.h"
#include "rpc.h"
#include "ui_voice.h"
#include "voice.h"

static const char *TAG = "playback";

/* Frame header, as on `audio_up`: stream id, flags, sequence (u16 LE). */
#define AUDIO_HEADER 4
#define FLAG_END 0x01
/* The host keeps about 0.5 s ahead; this takes a burst on top. */
/* A power of two, so the free-running indices wrap cleanly: 4.1 s. */
#define BUFFER_SAMPLES 65536u
/* Buffered before the speaker starts, so the first words don't stutter. */
#define PREBUFFER_MS 150
/* A stream whose end never came: the host went away. */
#define STARVED_MS 1500
#define CHUNK_SAMPLES 256
/* Silence written at the end to push the DMA tail out (6 x 240 frames). */
#define FLUSH_CHUNKS 6
#define IDLE_WAIT_MS 100
/* Level shown on the ring: this range of dBFS, updated this often. */
#define LEVEL_FLOOR_DB -45.0f
#define LEVEL_CEIL_DB -12.0f
#define LEVEL_EVERY_MS 100
#define TASK_CORE 1
/* Above the voice tasks: an underrun is heard, a late fetch isn't. */
#define TASK_PRIORITY 6
/* Earcons: sine notes with this much fade in and out, at this amplitude
 * (the speaker volume applies on top). */
#define EARCON_FADE_MS 8
#define EARCON_AMPLITUDE 9000.0f

typedef enum {
    END_DONE,
    END_STOPPED,
    END_REPLACED,
    END_STARVED,
    END_BARGE_IN,
} end_reason_t;

static const char *const END_NAMES[] = {
    [END_DONE] = "done",       [END_STOPPED] = "stopped",   [END_REPLACED] = "replaced",
    [END_STARVED] = "starved", [END_BARGE_IN] = "barge_in",
};

typedef struct {
    float hz; /* 0: a pause */
    int ms;
} note_t;

static const note_t ERROR_NOTES[] = {{587.3f, 110}, {0, 40}, {392.0f, 200}};
/* C6 E6 G6, twice. */
static const note_t ALARM_NOTES[] = {
    {1046.5f, 110}, {0, 30}, {1318.5f, 110}, {0, 30}, {1568.0f, 220}, {0, 160},
    {1046.5f, 110}, {0, 30}, {1318.5f, 110}, {0, 30}, {1568.0f, 220},
};

typedef struct {
    bool active;   /* frames of `id` are coming or buffered */
    bool end;      /* its last frame arrived */
    bool local;    /* an earcon, not the host's */
    uint8_t id;
    uint16_t next_seq;
    uint32_t received; /* samples */
    uint32_t played;
    uint32_t lost;     /* frames missing from the sequence */
    uint32_t overflow; /* samples with no room in the buffer */
    uint32_t underruns;
} stream_t;

/* Written by the link task, read by the playback task, both under s_lock. */
static SemaphoreHandle_t s_lock;
static int16_t *s_buf;
static uint32_t s_read;
static uint32_t s_write;
static stream_t s_stream;
/* A stream ended by the link task (replaced, stopped) for the playback
 * task to report. */
static stream_t s_ended;
static end_reason_t s_ended_reason;
static bool s_ended_pending;
/* The last stream that ended, whose late frames are dropped. */
static int s_last_id = -1;
static TaskHandle_t s_task;
static bool s_available;

bool playback_available(void)
{
    return s_available;
}

bool playback_active(void)
{
    return s_available && s_stream.active;
}

static uint32_t buffered(void)
{
    return s_write - s_read;
}

/** With s_lock held: hand the stream to the task to report, empty the buffer. */
static void end_locked(end_reason_t reason)
{
    if (!s_stream.active) {
        return;
    }
    s_ended = s_stream;
    s_ended_reason = reason;
    s_ended_pending = true;
    s_stream.active = false;
    if (!s_stream.local) {
        s_last_id = s_stream.id;
    }
    s_read = s_write;
}

void playback_receive(uint8_t *payload, size_t len)
{
    if (!s_available || len < AUDIO_HEADER || (len - AUDIO_HEADER) % 2 != 0) {
        return;
    }
    uint8_t id = payload[0];
    uint8_t flags = payload[1];
    uint16_t seq = (uint16_t) (payload[2] | (payload[3] << 8));
    const int16_t *pcm = (const int16_t *) (payload + AUDIO_HEADER);
    uint32_t n = (uint32_t) (len - AUDIO_HEADER) / 2;

    xSemaphoreTake(s_lock, portMAX_DELAY);
    stream_t *st = &s_stream;
    if (!st->active || st->local || st->id != id) {
        if (st->active) {
            end_locked(END_REPLACED);
        } else if (id == s_last_id) {
            // The tail of a stream that has ended already (stopped).
            xSemaphoreGive(s_lock);
            return;
        }
        memset(st, 0, sizeof(*st));
        st->active = true;
        st->id = id;
    }
    uint16_t gap = (uint16_t) (seq - st->next_seq);
    if (gap < 0x8000) {
        st->lost += gap;
        st->next_seq = (uint16_t) (seq + 1);
        uint32_t room = BUFFER_SAMPLES - buffered();
        uint32_t take = n < room ? n : room;
        for (uint32_t i = 0; i < take; i++) {
            s_buf[(s_write + i) % BUFFER_SAMPLES] = pcm[i];
        }
        s_write += take;
        st->received += take;
        st->overflow += n - take;
        if (flags & FLAG_END) {
            st->end = true;
        }
    }
    xSemaphoreGive(s_lock);
    xTaskNotifyGive(s_task);
}

static void stop(end_reason_t reason)
{
    if (!s_available) {
        return;
    }
    xSemaphoreTake(s_lock, portMAX_DELAY);
    end_locked(reason);
    xSemaphoreGive(s_lock);
    xTaskNotifyGive(s_task);
}

void playback_stop(void)
{
    stop(END_STOPPED);
}

void playback_barge_in(void)
{
    stop(END_BARGE_IN);
}

void playback_earcon(playback_earcon_t earcon)
{
    const note_t *notes = ERROR_NOTES;
    size_t count = sizeof(ERROR_NOTES) / sizeof(ERROR_NOTES[0]);
    if (earcon == PLAYBACK_EARCON_ALARM) {
        notes = ALARM_NOTES;
        count = sizeof(ALARM_NOTES) / sizeof(ALARM_NOTES[0]);
    }
    if (!s_available) {
        return;
    }
    xSemaphoreTake(s_lock, portMAX_DELAY);
    stream_t *st = &s_stream;
    if (st->active) {
        // Speech (or another earcon) is playing: it says enough.
        xSemaphoreGive(s_lock);
        return;
    }
    memset(st, 0, sizeof(*st));
    st->active = true;
    st->local = true;
    st->end = true;
    const uint32_t rate_ms = BOARD_AUDIO_SAMPLE_RATE / 1000;
    const uint32_t fade = EARCON_FADE_MS * rate_ms;
    for (size_t i = 0; i < count; i++) {
        uint32_t n = (uint32_t) notes[i].ms * rate_ms;
        if (n > BUFFER_SAMPLES - buffered()) {
            break;
        }
        for (uint32_t k = 0; k < n; k++) {
            float gain = 1.0f;
            if (k < fade) {
                gain = (float) k / fade;
            } else if (n - k < fade) {
                gain = (float) (n - k) / fade;
            }
            float v = notes[i].hz > 0 ? EARCON_AMPLITUDE * gain * sinf(2.0f * (float) M_PI * notes[i].hz * k / BOARD_AUDIO_SAMPLE_RATE) : 0;
            s_buf[(s_write + k) % BUFFER_SAMPLES] = (int16_t) v;
        }
        s_write += n;
        st->received += n;
    }
    xSemaphoreGive(s_lock);
    xTaskNotifyGive(s_task);
}

static void report(const stream_t *st, end_reason_t reason)
{
    if (st->local) {
        return;
    }
    int ms = (int) (st->played / (BOARD_AUDIO_SAMPLE_RATE / 1000));
    ESP_LOGI(TAG, "stream %d: %d ms, %s, %lu lost, %lu underruns", st->id, ms, END_NAMES[reason],
             (unsigned long) st->lost, (unsigned long) st->underruns);
    cJSON *params = cJSON_CreateObject();
    cJSON_AddNumberToObject(params, "id", st->id);
    cJSON_AddStringToObject(params, "reason", END_NAMES[reason]);
    cJSON_AddNumberToObject(params, "ms", ms);
    cJSON_AddNumberToObject(params, "lost", st->lost);
    cJSON_AddNumberToObject(params, "overflow", st->overflow);
    cJSON_AddNumberToObject(params, "underruns", st->underruns);
    rpc_notify("playback_end", params);
}

static void set_level(float level)
{
    lvgl_port_lock();
    ui_voice_set_level(level < 0 ? 0 : level > 1 ? 1 : level);
    lvgl_port_unlock();
}

/** Speaker on, eyes speaking (not for an earcon), the wake word ignored
 * or barging in. */
static void begin(bool local)
{
    voice_set_speaking(true);
    board_audio_set_mute(false);
    if (!local) {
        voice_set_state(VOICE_SPEAKING);
    }
}

static void finish(void)
{
    int16_t silence[CHUNK_SAMPLES] = {0};
    for (int i = 0; i < FLUSH_CHUNKS; i++) {
        board_audio_write(silence, CHUNK_SAMPLES);
    }
    board_audio_set_mute(true);
    set_level(0);
    if (voice_state() == VOICE_SPEAKING) {
        voice_set_state(VOICE_IDLE);
    }
    voice_set_speaking(false);
}

static void playback_task(void *arg)
{
    int16_t chunk[CHUNK_SAMPLES];
    bool playing = false;
    int64_t starved_since = 0;
    int64_t level_us = 0;
    float level_sum = 0;
    uint32_t level_n = 0;
    while (true) {
        if (!playing) {
            ulTaskNotifyTake(pdTRUE, pdMS_TO_TICKS(IDLE_WAIT_MS));
        }
        xSemaphoreTake(s_lock, portMAX_DELAY);
        bool ended = s_ended_pending;
        stream_t done = s_ended;
        end_reason_t why = s_ended_reason;
        s_ended_pending = false;
        stream_t *st = &s_stream;
        uint32_t n = 0;
        bool start = !playing && st->active && (buffered() >= PREBUFFER_MS * (BOARD_AUDIO_SAMPLE_RATE / 1000) || st->end);
        if (playing || start) {
            n = buffered() < CHUNK_SAMPLES ? buffered() : CHUNK_SAMPLES;
            for (uint32_t i = 0; i < n; i++) {
                chunk[i] = s_buf[(s_read + i) % BUFFER_SAMPLES];
            }
            s_read += n;
            st->played += n;
        }
        bool drained = st->active && st->end && buffered() == 0 && n == 0;
        stream_t now = *st;
        if (drained) {
            st->active = false;
            if (!st->local) {
                s_last_id = st->id;
            }
        }
        xSemaphoreGive(s_lock);

        if (ended) {
            // Stopped or replaced: the speaker stays on for the next stream.
            report(&done, why);
            if (playing && !now.active) {
                finish();
                playing = false;
            }
        }
        if (start) {
            begin(now.local);
            playing = true;
            starved_since = 0;
        }
        if (!playing) {
            continue;
        }
        if (drained) {
            finish();
            playing = false;
            report(&now, END_DONE);
            continue;
        }
        int64_t t = esp_timer_get_time();
        if (n == 0) {
            // Nothing buffered and no end yet: keep the DMA fed with silence.
            memset(chunk, 0, sizeof(chunk));
            n = CHUNK_SAMPLES;
            if (starved_since == 0) {
                starved_since = t;
                xSemaphoreTake(s_lock, portMAX_DELAY);
                s_stream.underruns++;
                xSemaphoreGive(s_lock);
            } else if ((t - starved_since) / 1000 >= STARVED_MS) {
                xSemaphoreTake(s_lock, portMAX_DELAY);
                end_locked(END_STARVED);
                xSemaphoreGive(s_lock);
                continue;
            }
        } else {
            starved_since = 0;
        }
        for (uint32_t i = 0; i < n; i++) {
            level_sum += (float) chunk[i] * chunk[i];
        }
        level_n += n;
        if (t - level_us >= LEVEL_EVERY_MS * 1000 && level_n > 0) {
            float rms = sqrtf(level_sum / level_n);
            float db = rms > 0 ? 20.0f * log10f(rms / 32768.0f) : LEVEL_FLOOR_DB;
            set_level((db - LEVEL_FLOOR_DB) / (LEVEL_CEIL_DB - LEVEL_FLOOR_DB));
            level_us = t;
            level_sum = 0;
            level_n = 0;
        }
        board_audio_write(chunk, n);
    }
}

void playback_start(void)
{
    s_lock = xSemaphoreCreateMutex();
    s_buf = heap_caps_malloc(BUFFER_SAMPLES * sizeof(int16_t), MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    if (s_lock == NULL || s_buf == NULL) {
        ESP_LOGE(TAG, "out of memory, continuing without playback");
        return;
    }
    if (xTaskCreatePinnedToCore(playback_task, "playback", 4096, NULL, TASK_PRIORITY, &s_task, TASK_CORE) != pdPASS) {
        ESP_LOGE(TAG, "playback task not started");
        return;
    }
    s_available = true;
    ESP_LOGI(TAG, "ready, %d ms buffer", (int) (BUFFER_SAMPLES / (BOARD_AUDIO_SAMPLE_RATE / 1000)));
}
