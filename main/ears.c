#include "ears.h"

#include <math.h>

#include "esp_log.h"
#include "esp_timer.h"
#include "pet.h"

static const char *TAG = "ears";

#define RATE_PER_MS 16
/* A sound starts when a chunk's peak rises this far over the room's level,
 * and this loud at least; it's over when the chunk's level falls back to
 * within END_DB of the room's. */
#define ONSET_DB 24.0f
#define ONSET_MIN_DB -32.0f
#define END_DB 12.0f
/* A clap (or a knock) is over this soon; longer, and this loud, it's a bang
 * (a door, something falling); longer still, a noise that goes on (a
 * vacuum, a blender) and nothing to react to. */
#define CLAP_MAX_MS 160
#define BANG_MAX_MS 1200
#define BANG_DB -8.0f
/* Claps in a row are this far apart; a sound sooner after the last is its
 * echo, and the row is over this long after the last. */
#define CLAP_GAP_MIN_MS 110
#define CLAP_GAP_MAX_MS 750
/* More than this in a row is someone hammering, typing hard, or talking. */
#define CLAPS_MAX 6
/* The room's level follows a quieter one in about a second, a louder one in
 * half a minute: a sound doesn't become the background. */
#define FLOOR_DOWN_MS 1000.0f
#define FLOOR_UP_MS 30000.0f
/* Louder than the room by this (and than QUIET_DB) is a sound that breaks
 * the quiet. */
#define SOUND_DB 10.0f
#define QUIET_DB -62.0f
/* Talk heard over the last couple of minutes (decaying), for chatter. */
#define TALK_TAU_MS 90000.0f
#define CHATTER_ON_MS 20000.0f
#define CHATTER_OFF_MS 8000.0f
/* After the board's own sound, its echo in the room. */
#define DEAF_TAIL_MS 500
/* Not fed for this long: the mic isn't listened to (muted, paused). */
#define STALE_MS 2000
#define MIN_DB -96.0f

static volatile bool s_on = true;

/* All but the snapshot below only touched by the fetch task. */
static bool s_started;
static float s_floor = -60.0f;
static int64_t s_deaf_until_us;
/* A sound going on, since when and how loud at its peak. */
static bool s_loud;
static int64_t s_onset_us;
static float s_onset_peak;
/* The row of claps: how many, when the last ended, the loudest, and whether
 * someone talked meanwhile. */
static int s_row;
static int64_t s_row_end_us;
static float s_row_peak;
static bool s_row_bang;
static bool s_row_speech;
static float s_talk_ms;
static bool s_chatter;
static int64_t s_sound_us;

/* For ears_get(), from other tasks. */
static volatile int64_t s_fed_us;
static volatile float s_level;

static float db(float v)
{
    return v > 0 ? 10.0f * log10f(v) : MIN_DB;
}

static void row_reset(void)
{
    s_row = 0;
    s_row_peak = MIN_DB;
    s_row_bang = false;
    s_row_speech = false;
}

/** The row of sounds is over: what was it? */
static void row_over(void)
{
    if (s_row_speech || s_row > CLAPS_MAX) {
        ESP_LOGD(TAG, "%d sounds with speech or too many: not claps", s_row);
    } else if (s_row >= 2) {
        ESP_LOGI(TAG, "%d claps (peak %.0f dBFS, room %.0f)", s_row, s_row_peak, s_floor);
        pet_heard(PET_HEARD_CLAPS, s_row);
    } else if (s_row == 1 && (s_row_bang || s_row_peak >= BANG_DB)) {
        ESP_LOGI(TAG, "bang (peak %.0f dBFS, room %.0f)", s_row_peak, s_floor);
        pet_heard(PET_HEARD_BANG, 1);
    }
    row_reset();
}

/** A sound `ms` long ended at `now`. */
static void sound_over(int64_t now, int ms)
{
    if (ms > BANG_MAX_MS) {
        // Something that goes on: a blender, not a clap.
        row_reset();
        return;
    }
    if (ms > CLAP_MAX_MS) {
        // A bang stands alone: whatever was in the row, it's over with it.
        if (s_row > 0) {
            row_over();
        }
        s_row = 1;
        s_row_peak = s_onset_peak;
        s_row_bang = s_onset_peak >= BANG_DB;
        s_row_end_us = now;
        row_over();
        return;
    }
    s_row++;
    s_row_end_us = now;
    if (s_onset_peak > s_row_peak) {
        s_row_peak = s_onset_peak;
    }
}

void ears_feed(const int16_t *pcm, int samples, bool speech, bool deaf)
{
    if (!s_on || samples <= 0) {
        return;
    }
    int64_t now = esp_timer_get_time();
    s_fed_us = now;
    float ms = (float) samples / RATE_PER_MS;

    int64_t sum = 0;
    int peak = 0;
    for (int i = 0; i < samples; i++) {
        int v = pcm[i];
        sum += (int64_t) v * v;
        if (v < 0) {
            v = -v;
        }
        if (v > peak) {
            peak = v;
        }
    }
    float level = db((float) sum / samples / (32768.0f * 32768.0f));
    float peak_db = db((float) peak * peak / (32768.0f * 32768.0f));
    s_level = level;

    if (deaf) {
        s_deaf_until_us = now + DEAF_TAIL_MS * 1000;
    }
    if (now < s_deaf_until_us) {
        // The board's own voice: forget any sound going on.
        s_loud = false;
        row_reset();
        return;
    }
    if (!s_started) {
        s_started = true;
        s_floor = level;
        s_sound_us = now;
    }

    // The room's level, slow to rise so a sound doesn't become it.
    if (!s_loud) {
        float tau = level < s_floor ? FLOOR_DOWN_MS : FLOOR_UP_MS;
        s_floor += (level - s_floor) * fminf(1.0f, ms / tau);
    }

    if (speech) {
        if (s_row > 0 || s_loud) {
            s_row_speech = true;
        }
        s_talk_ms += ms;
        s_sound_us = now;
    }
    s_talk_ms -= s_talk_ms * ms / TALK_TAU_MS;
    if (!s_chatter && s_talk_ms >= CHATTER_ON_MS) {
        s_chatter = true;
        ESP_LOGI(TAG, "people talking nearby");
        pet_heard(PET_HEARD_CHATTER, 1);
    } else if (s_chatter && s_talk_ms < CHATTER_OFF_MS) {
        s_chatter = false;
    }
    if (level >= s_floor + SOUND_DB && level >= QUIET_DB) {
        s_sound_us = now;
    }

    if (!s_loud) {
        if (peak_db >= s_floor + ONSET_DB && peak_db >= ONSET_MIN_DB) {
            if (s_row > 0 && (now - s_row_end_us) / 1000 < CLAP_GAP_MIN_MS) {
                // The last one's echo, or the same sound twice: one clap.
                s_row--;
            }
            if (s_row == 0) {
                s_row_speech = speech;
            }
            s_loud = true;
            s_onset_us = now;
            s_onset_peak = peak_db;
        } else if (s_row > 0 && (now - s_row_end_us) / 1000 > CLAP_GAP_MAX_MS) {
            row_over();
        }
    } else {
        if (peak_db > s_onset_peak) {
            s_onset_peak = peak_db;
        }
        int long_ms = (int) ((now - s_onset_us) / 1000);
        if (level < s_floor + END_DB) {
            s_loud = false;
            sound_over(now, long_ms);
        } else if (long_ms > BANG_MAX_MS) {
            s_loud = false;
            sound_over(now, long_ms);
        }
    }
}

void ears_set_on(bool on)
{
    s_on = on;
}

bool ears_on(void)
{
    return s_on;
}

void ears_get(ears_state_t *out)
{
    int64_t now = esp_timer_get_time();
    out->listening = s_on && s_started && now - s_fed_us < (int64_t) STALE_MS * 1000;
    out->floor_db = s_floor;
    out->level_db = s_level;
    out->quiet_s = out->listening ? (uint32_t) ((now - s_sound_us) / 1000000) : 0;
    out->chatter = out->listening && s_chatter;
}
