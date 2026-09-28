#include "audio_selftest.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "board_audio.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/task.h"
#include "sdkconfig.h"

static const char *TAG = "audio_selftest";

#define CMD_MAX 64
#define CHUNK_FRAMES 256
#define REC_MAX_SECONDS 8
#define TONE_AMPLITUDE 8000.0f
#define TEST_TONE_HZ 1000
#define DUMP_BYTES_PER_LINE 576 /* multiple of 3: no base64 padding mid-stream */

static QueueHandle_t s_queue;
static bool s_ready;

/* Last recording, BOARD_AUDIO_IN_CHANNELS interleaved, in PSRAM. */
static int16_t *s_rec;
static size_t s_rec_frames;

static void reply_ok(const char *cmd)
{
    printf("AUD:OK %s\n", cmd);
    fflush(stdout);
}

static void reply_err(const char *why)
{
    printf("AUD:ERR %s\n", why);
    fflush(stdout);
}

/** Next chunk of a sine at hz with a 5 ms fade in/out, so notes don't click. */
static void sine_chunk(int16_t *out, size_t n, int hz, size_t *pos, size_t total)
{
    const size_t fade = BOARD_AUDIO_SAMPLE_RATE / 200;
    for (size_t i = 0; i < n; i++, (*pos)++) {
        float env = 1.0f;
        if (*pos < fade) {
            env = (float) *pos / fade;
        } else if (total - *pos < fade) {
            env = (float) (total - *pos) / fade;
        }
        float t = (float) *pos / BOARD_AUDIO_SAMPLE_RATE;
        out[i] = (int16_t) (TONE_AMPLITUDE * env * sinf(2.0f * (float) M_PI * hz * t));
    }
}

static esp_err_t play_sine(int hz, int ms)
{
    int16_t buf[CHUNK_FRAMES];
    size_t total = (size_t) BOARD_AUDIO_SAMPLE_RATE * ms / 1000;
    size_t pos = 0;
    while (pos < total) {
        size_t n = total - pos < CHUNK_FRAMES ? total - pos : CHUNK_FRAMES;
        sine_chunk(buf, n, hz, &pos, total);
        if (board_audio_write(buf, n) != ESP_OK) {
            return ESP_FAIL;
        }
    }
    return ESP_OK;
}

/** Push out the DMA tail, then mute so the PA doesn't hiss between tests. */
static void flush_and_mute(void)
{
    int16_t silence[CHUNK_FRAMES] = {0};
    for (int i = 0; i < 6; i++) {
        board_audio_write(silence, CHUNK_FRAMES);
    }
    board_audio_set_mute(true);
}

static void cmd_tone(const char *args)
{
    int hz = 0, ms = 0;
    board_audio_set_mute(false);
    esp_err_t err;
    if (sscanf(args, "%d %d", &hz, &ms) == 2 && hz > 0 && hz < 8000 && ms > 0 && ms <= 5000) {
        err = play_sine(hz, ms);
    } else {
        // Two-note chime: E6 then B6.
        err = play_sine(1319, 120);
        if (err == ESP_OK) {
            err = play_sine(1976, 220);
        }
    }
    flush_and_mute();
    if (err == ESP_OK) {
        reply_ok("tone");
    } else {
        reply_err("write failed");
    }
}

static uint32_t crc32_update(uint32_t crc, const uint8_t *p, size_t n)
{
    crc = ~crc;
    while (n--) {
        crc ^= *p++;
        for (int k = 0; k < 8; k++) {
            crc = (crc >> 1) ^ (0xEDB88320u & (0u - (crc & 1u)));
        }
    }
    return ~crc;
}

static size_t base64_encode(const uint8_t *in, size_t n, char *out)
{
    static const char tbl[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    size_t o = 0;
    for (size_t i = 0; i < n; i += 3) {
        uint32_t v = (uint32_t) in[i] << 16;
        if (i + 1 < n) {
            v |= (uint32_t) in[i + 1] << 8;
        }
        if (i + 2 < n) {
            v |= in[i + 2];
        }
        out[o++] = tbl[(v >> 18) & 63];
        out[o++] = tbl[(v >> 12) & 63];
        out[o++] = i + 1 < n ? tbl[(v >> 6) & 63] : '=';
        out[o++] = i + 2 < n ? tbl[v & 63] : '=';
    }
    return o;
}

static void dump_recording(void)
{
    const uint8_t *bytes = (const uint8_t *) s_rec;
    size_t len = s_rec_frames * BOARD_AUDIO_IN_CHANNELS * sizeof(int16_t);
    printf("AUD:BEGIN rate=%d channels=%d frames=%u bytes=%u\n", BOARD_AUDIO_SAMPLE_RATE,
           BOARD_AUDIO_IN_CHANNELS, (unsigned) s_rec_frames, (unsigned) len);

    // One fwrite per line, so a log line from another task can't split it.
    static char line[8 + DUMP_BYTES_PER_LINE / 3 * 4 + 2];
    uint32_t crc = 0;
    for (size_t off = 0; off < len; off += DUMP_BYTES_PER_LINE) {
        size_t n = len - off < DUMP_BYTES_PER_LINE ? len - off : DUMP_BYTES_PER_LINE;
        crc = crc32_update(crc, bytes + off, n);
        memcpy(line, "AUD:D ", 6);
        size_t o = 6 + base64_encode(bytes + off, n, line + 6);
        line[o++] = '\n';
        fwrite(line, 1, o, stdout);
    }
    printf("AUD:END crc32=%08lx\n", (unsigned long) crc);
    fflush(stdout);
}

static void cmd_rec(const char *args)
{
    int seconds = 0;
    char opt[8] = "";
    if (sscanf(args, "%d %7s", &seconds, opt) < 1 || seconds < 1 || seconds > REC_MAX_SECONDS) {
        reply_err("usage: rec 1..8 [tone|beep]");
        return;
    }
    bool tone = strcmp(opt, "tone") == 0;
    if (strcmp(opt, "beep") == 0) {
        // "Speak now" cue for a person at the board, who can't see the host.
        board_audio_set_mute(false);
        play_sine(880, 150);
        flush_and_mute();
    }

    size_t frames = (size_t) seconds * BOARD_AUDIO_SAMPLE_RATE;
    size_t bytes = frames * BOARD_AUDIO_IN_CHANNELS * sizeof(int16_t);
    if (s_rec == NULL) {
        s_rec = heap_caps_malloc(REC_MAX_SECONDS * BOARD_AUDIO_SAMPLE_RATE * BOARD_AUDIO_IN_CHANNELS
                                     * sizeof(int16_t),
                                 MALLOC_CAP_SPIRAM);
        if (s_rec == NULL) {
            reply_err("no PSRAM for the recording");
            return;
        }
    }
    memset(s_rec, 0, bytes);

    if (tone) {
        board_audio_set_mute(false);
    }
    int16_t out[CHUNK_FRAMES];
    size_t tone_pos = 0;
    esp_err_t err = ESP_OK;
    // Speaker and mic run off the same I2S clock, so one write per read keeps them in step.
    for (size_t done = 0; done < frames && err == ESP_OK; done += CHUNK_FRAMES) {
        size_t n = frames - done < CHUNK_FRAMES ? frames - done : CHUNK_FRAMES;
        if (tone) {
            sine_chunk(out, n, TEST_TONE_HZ, &tone_pos, frames);
            err = board_audio_write(out, n);
        }
        if (err == ESP_OK) {
            err = board_audio_read(s_rec + done * BOARD_AUDIO_IN_CHANNELS, n);
        }
    }
    if (tone) {
        flush_and_mute();
    }
    if (err != ESP_OK) {
        s_rec_frames = 0;
        reply_err("i2s read/write failed");
        return;
    }
    s_rec_frames = frames;
    dump_recording();
    reply_ok("rec");
}

static void cmd_play(const char *args)
{
    int ch = 0;
    sscanf(args, "%d", &ch);
    if (s_rec_frames == 0) {
        reply_err("nothing recorded yet");
        return;
    }
    if (ch < 0 || ch >= BOARD_AUDIO_IN_CHANNELS) {
        reply_err("channel out of range");
        return;
    }
    board_audio_set_mute(false);
    int16_t buf[CHUNK_FRAMES];
    esp_err_t err = ESP_OK;
    for (size_t done = 0; done < s_rec_frames && err == ESP_OK; done += CHUNK_FRAMES) {
        size_t n = s_rec_frames - done < CHUNK_FRAMES ? s_rec_frames - done : CHUNK_FRAMES;
        for (size_t i = 0; i < n; i++) {
            buf[i] = s_rec[(done + i) * BOARD_AUDIO_IN_CHANNELS + ch];
        }
        err = board_audio_write(buf, n);
    }
    flush_and_mute();
    if (err == ESP_OK) {
        reply_ok("play");
    } else {
        reply_err("write failed");
    }
}

static void cmd_stats(void)
{
    printf("AUD:STAT heap internal free=%u min=%u largest=%u\n",
           (unsigned) heap_caps_get_free_size(MALLOC_CAP_INTERNAL),
           (unsigned) heap_caps_get_minimum_free_size(MALLOC_CAP_INTERNAL),
           (unsigned) heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL));
    printf("AUD:STAT heap psram free=%u min=%u\n", (unsigned) heap_caps_get_free_size(MALLOC_CAP_SPIRAM),
           (unsigned) heap_caps_get_minimum_free_size(MALLOC_CAP_SPIRAM));
#if CONFIG_FREERTOS_USE_TRACE_FACILITY && CONFIG_FREERTOS_GENERATE_RUN_TIME_STATS
    // Two snapshots one second apart: CPU share per task over that second.
    UBaseType_t cap = uxTaskGetNumberOfTasks() + 4;
    TaskStatus_t *a = calloc(cap, sizeof(TaskStatus_t));
    TaskStatus_t *b = calloc(cap, sizeof(TaskStatus_t));
    if (a == NULL || b == NULL) {
        free(a);
        free(b);
        reply_err("no memory for task stats");
        return;
    }
    configRUN_TIME_COUNTER_TYPE t0 = 0, t1 = 0;
    UBaseType_t na = uxTaskGetSystemState(a, cap, &t0);
    vTaskDelay(pdMS_TO_TICKS(1000));
    UBaseType_t nb = uxTaskGetSystemState(b, cap, &t1);
    configRUN_TIME_COUNTER_TYPE span = t1 - t0;
    for (UBaseType_t i = 0; i < nb && span > 0; i++) {
        for (UBaseType_t j = 0; j < na; j++) {
            if (a[j].xHandle == b[i].xHandle) {
                configRUN_TIME_COUNTER_TYPE d = b[i].ulRunTimeCounter - a[j].ulRunTimeCounter;
                printf("AUD:STAT task %-16s cpu=%5.1f%% stack_free=%u\n", b[i].pcTaskName,
                       100.0 * (double) d / (double) span, (unsigned) b[i].usStackHighWaterMark);
                break;
            }
        }
    }
    free(a);
    free(b);
#else
    printf("AUD:STAT task stats disabled (CONFIG_FREERTOS_GENERATE_RUN_TIME_STATS)\n");
#endif
    reply_ok("stats");
}

static void run(const char *cmd)
{
    const char *args = strchr(cmd, ' ');
    args = args != NULL ? args + 1 : "";
    if (strncmp(cmd, "tone", 4) == 0) {
        cmd_tone(args);
    } else if (strncmp(cmd, "rec", 3) == 0) {
        cmd_rec(args);
    } else if (strncmp(cmd, "play", 4) == 0) {
        cmd_play(args);
    } else if (strncmp(cmd, "vol", 3) == 0) {
        int v = atoi(args);
        if (v < 0 || v > 100 || board_audio_set_volume(v) != ESP_OK) {
            reply_err("usage: vol 0..100");
        } else {
            reply_ok("vol");
        }
    } else if (strncmp(cmd, "gain", 4) == 0) {
        float db = strtof(args, NULL);
        if (db < 0.0f || db > 37.5f || board_audio_set_in_gain(db) != ESP_OK) {
            reply_err("usage: gain 0..37.5");
        } else {
            reply_ok("gain");
        }
    } else {
        reply_err("unknown command");
    }
}

static void selftest_task(void *arg)
{
    char cmd[CMD_MAX];
    while (true) {
        if (xQueueReceive(s_queue, cmd, portMAX_DELAY) == pdTRUE) {
            run(cmd);
        }
    }
}

bool audio_selftest_command(const char *line)
{
    size_t plen = sizeof(AUDIO_SELFTEST_PREFIX) - 1;
    if (strncmp(line, AUDIO_SELFTEST_PREFIX, plen) != 0) {
        return false;
    }
    const char *cmd = line + plen;
    while (*cmd == ' ') {
        cmd++;
    }
    if (!s_ready) {
        reply_err("audio not available");
        return true;
    }
    // Stats run right here so they can watch the self-test task while it records or plays.
    if (strncmp(cmd, "stats", 5) == 0) {
        cmd_stats();
        return true;
    }
    char buf[CMD_MAX];
    strlcpy(buf, cmd, sizeof(buf));
    if (xQueueSend(s_queue, buf, 0) != pdTRUE) {
        reply_err("busy");
    }
    return true;
}

void audio_selftest_start(void)
{
    if (board_audio_init() != ESP_OK) {
        ESP_LOGE(TAG, "audio init failed, continuing without audio");
        return;
    }
    s_queue = xQueueCreate(4, CMD_MAX);
    // Core 1, above LVGL (5): I2S must be fed on time or the speaker glitches.
    if (s_queue == NULL
        || xTaskCreatePinnedToCore(selftest_task, "audio_test", 4096, NULL, 6, NULL, 1) != pdPASS) {
        ESP_LOGE(TAG, "failed to start the self-test task");
        return;
    }
    s_ready = true;
#if CONFIG_DUALEYE_AUDIO_BOOT_CHIME
    char chime[CMD_MAX] = "tone";
    xQueueSend(s_queue, chime, 0);
#endif
}
