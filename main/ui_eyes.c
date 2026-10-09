#include "ui_eyes.h"

#include <math.h>
#include <string.h>

#include "esp_random.h"
#include "pet.h"
#include "playback.h"

/* 25 fps: both eyes repainted each frame stay well inside the SPI bus. */
#define FRAME_MS 40

#define LISTENING_COLOR 0x30D5F0
#define THINKING_COLOR 0xFFB020
#define SPEAKING_COLOR 0x40E080
#define ERROR_COLOR 0xFF4040

/* Each eye's shape chases its expression on springs: a little bouncy for
 * the shape, snappy and settled for where it looks. */
#define SHAPE_K 220.0f
#define SHAPE_ZETA 0.5f
#define GAZE_K 650.0f
#define GAZE_ZETA 0.85f
#define SPRING_STEP_S 0.01f
/* The colour fades to the new state's in about this many frames. */
#define COLOR_RATE 0.25f
/* Closing on idle: the eyes are gone (and the watch face back) once they are
 * this flat, at least this long after the state changed. */
#define CLOSED_H 12.0f
#define CLOSE_MIN_MS 260
/* A blink: down, held shut, up. */
#define BLINK_DOWN_MS 70
#define BLINK_HOLD_MS 30
#define BLINK_UP_MS 80
#define BLINK_SHUT 0.07f
/* Corner radius, as a share of the shorter side. */
#define RADIUS_SHARE 0.3f
/* The circle under a smiling eye, in tenths of the eye's width: wider is flatter. */
#define SMILE_SHARE 16

#define PI2 6.2831853f

/* While nobody is talking, a little scene every IDLE_MIN_S..IDLE_MAX_S
 * seconds of quiet (counted again after each conversation). */
#define IDLE_MIN_S 30
#define IDLE_MAX_S 120
#define IDLE_CHECK_MS 1000

/* The eyes face: a soft white, a dim blue as it dozes. The pointer leads the
 * eyes for POINTER_FRESH_MS after it last moved, then they glance about on
 * their own. They doze off once nothing has happened for DOZE_AFTER_MS (no
 * pointer, no conversation), over DOZE_MS, and wake when the pointer moves;
 * but only once the host has sent the pointer at all: without it they stay
 * awake. */
#define AMBIENT_COLOR 0xE6F2FF
#define ASLEEP_COLOR 0x34507A
#define POINTER_FRESH_MS 6000
#define DOZE_AFTER_MS 60000
#define DOZE_MS 20000
/* Fast asleep, it snores this many times, this far apart, then sleeps quietly. */
#define SNORES 3
#define SNORE_EVERY_MS 5200
/* How far the pointer moves the eyes, in pixels: up and down about a point a
 * little above the middle, the board usually sitting below the screen. */
#define LOOK_X 36.0f
#define LOOK_Y 26.0f
#define LOOK_Y_BIAS -8.0f

typedef enum { P_W, P_H, P_X, P_Y, P_LID_IN, P_LID_OUT, P_HAPPY, P_COUNT } param_t;

typedef struct {
    float v, vel;
} spring_t;

/* One frame of one eye, in screen pixels. */
typedef struct {
    int32_t x0, y0, x1, y1;
    int32_t radius;
    /* How far the top lid comes down at the left and right corners. */
    int32_t lid_left, lid_right;
    /* Top of the black arch that pushes the bottom up into a smile; 0 for none. */
    int32_t happy_top;
} geom_t;

typedef struct {
    /* The watch face, made transparent while the eyes are open so it isn't drawn under them. */
    lv_obj_t *screen;
    /* Black, full screen, on the top layer: the eye is drawn on it. */
    lv_obj_t *stage;
    spring_t p[P_COUNT];
    geom_t geom;
    bool drawn;
    /* The stage is up (the watch face hidden under it), since `shown_ms`. */
    bool shown;
    uint32_t shown_ms;
} eye_t;

static eye_t s_eyes[BOARD_LCD_COUNT];
static lv_timer_t *s_timer;
/* A conversation or a scene is on both screens, or still closing on one
 * that isn't showing the eyes face. */
static bool s_open;
static voice_state_t s_state = VOICE_IDLE;
static uint32_t s_state_ms;
static uint32_t s_last_ms;

static float s_level_in;
static float s_level;

static float s_rgb[3];
static lv_color_t s_color;

static uint32_t s_next_blink_ms;
static uint32_t s_blink_ms;
static bool s_blinking;
static bool s_blink_again;
static float s_blink = 1.0f;

static uint32_t s_next_glance_ms;
static float s_gaze_x;
static float s_gaze_y;

/* Screens showing the eyes face, and when it last had something to watch
 * besides the pointer (it came on, a conversation). */
static bool s_ambient[BOARD_LCD_COUNT];
static uint32_t s_awake_ms;
/* The host's pointer in thousandths of -1..1, from the link task; when it
 * last moved (lv_tick, 0 for never). */
static volatile int32_t s_ptr_x;
static volatile int32_t s_ptr_y;
static volatile uint32_t s_ptr_ms;
/* Where the eyes face looks, and how far asleep it is (0..1). */
static float s_look_x;
static float s_look_y;
static uint32_t s_next_look_ms;
static float s_doze;
static int s_snores;
static uint32_t s_next_snore_ms;

/* A sound in a scene, `ms` into it. */
typedef struct {
    uint16_t ms;
    sound_t sound;
} cue_t;

#define MAX_CUES 4

/* A scene the eyes play on their own while idle: what each eye looks like
 * `t` seconds in, for `ms`, in `color` (RAINBOW: going round the hues), and
 * what it says meanwhile. The cues are in order; SOUND_WAKE (0) ends them,
 * no scene plays it. */
typedef struct {
    const char *name;
    uint32_t color;
    uint32_t ms;
    bool blinks;
    void (*fn)(int eye, float t, float out[P_COUNT]);
    cue_t cues[MAX_CUES];
} skit_t;

#define RAINBOW 0

static const skit_t *s_skit;
/* The scene that just ended, its colour kept while the eyes close. */
static const skit_t *s_closing;
static const skit_t *s_last_skit;
static uint32_t s_skit_ms;
/* The scene's next cue. */
static int s_cue;
static lv_timer_t *s_idle_timer;
static bool s_idle_on;
static bool s_busy;
static uint32_t s_next_idle_ms;

static bool any_ambient(void)
{
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        if (s_ambient[i]) {
            return true;
        }
    }
    return false;
}

/* White awake (tinted by the pet's mood), a dim blue asleep. */
static uint32_t ambient_color(void)
{
    uint32_t awake = pet_tint(AMBIENT_COLOR);
    uint32_t c = 0;
    for (int shift = 0; shift <= 16; shift += 8) {
        float a = (float) ((awake >> shift) & 0xFF);
        float b = (float) ((ASLEEP_COLOR >> shift) & 0xFF);
        c |= (uint32_t) (a + (b - a) * s_doze + 0.5f) << shift;
    }
    return c;
}

/* Round the hues once every 3 s, bright. */
static uint32_t rainbow_color(void)
{
    float h = fmodf((float) lv_tick_get() / 3000.0f, 1.0f) * 6.0f;
    float x = 1.0f - fabsf(fmodf(h, 2.0f) - 1.0f);
    float rgb[3] = {0};
    int sector = (int) h;
    static const int8_t PICK[6][3] = {{0, 1, -1}, {1, 0, -1}, {-1, 0, 1}, {-1, 1, 0}, {1, -1, 0}, {0, -1, 1}};
    for (int c = 0; c < 3; c++) {
        int8_t which = PICK[sector % 6][c];
        rgb[c] = which == 0 ? 1.0f : which == 1 ? x : 0.0f;
    }
    // Never quite black: the eyes stay eyes.
    uint32_t out = 0;
    for (int c = 0; c < 3; c++) {
        out = (out << 8) | (uint32_t) (60.0f + 195.0f * rgb[c]);
    }
    return out;
}

static uint32_t state_color(voice_state_t state)
{
    const skit_t *skit = s_skit != NULL ? s_skit : s_closing;
    if (skit != NULL && state == VOICE_IDLE) {
        return skit->color == RAINBOW ? rainbow_color() : skit->color;
    }
    if (state == VOICE_IDLE && any_ambient()) {
        return ambient_color();
    }
    switch (state) {
    case VOICE_THINKING:
        return THINKING_COLOR;
    case VOICE_SPEAKING:
        return SPEAKING_COLOR;
    case VOICE_ERROR:
        return ERROR_COLOR;
    default:
        return LISTENING_COLOR;
    }
}

/* The idle scenes. Each sets the targets the springs chase, so a jump from
 * one pose to the next comes out as a bouncy move, not a cut. Eye 0 is the
 * left screen: +x is towards its inner corner, -x towards eye 1's. */

static void open_pose(float out[P_COUNT])
{
    out[P_W] = 110.0f;
    out[P_H] = 124.0f;
}

static void skit_look_around(int eye, float t, float out[P_COUNT])
{
    // Left, right, a peek up, back to you.
    open_pose(out);
    out[P_X] = t < 0.8f ? 0 : t < 1.9f ? -36.0f : t < 3.0f ? 36.0f : t < 3.7f ? 10.0f : 0;
    out[P_Y] = t >= 3.0f && t < 3.7f ? -26.0f : t >= 0.8f && t < 3.0f ? 4.0f : 0;
}

static void skit_sleepy(int eye, float t, float out[P_COUNT])
{
    // Drooping lids, nodding off, a start, drooping again.
    open_pose(out);
    if (t < 3.4f) {
        float d = fminf(t / 3.4f, 1.0f);
        out[P_H] = 104.0f - 80.0f * d;
        out[P_LID_IN] = out[P_LID_OUT] = 0.35f + 0.25f * d;
        out[P_Y] = 6.0f + 10.0f * d;
    } else if (t < 4.2f) {
        out[P_W] = 120.0f;
        out[P_H] = 146.0f;
        out[P_Y] = -6.0f;
    } else {
        out[P_H] = 96.0f;
        out[P_LID_IN] = out[P_LID_OUT] = 0.4f;
        out[P_Y] = 6.0f;
    }
}

static void skit_suspicious(int eye, float t, float out[P_COUNT])
{
    // Narrowed, flat lids, shifty looks; one eye narrower.
    out[P_W] = 114.0f;
    out[P_H] = eye == 0 ? 64.0f : 78.0f;
    out[P_LID_IN] = out[P_LID_OUT] = 0.2f;
    out[P_X] = t < 0.5f ? 0 : t < 1.8f ? -30.0f : t < 3.1f ? 30.0f : t < 3.6f ? -30.0f : 0;
}

static void skit_happy(int eye, float t, float out[P_COUNT])
{
    // Smiling and bouncing.
    out[P_W] = 118.0f;
    out[P_H] = 116.0f;
    out[P_HAPPY] = 0.42f;
    out[P_Y] = -6.0f - 10.0f * fabsf(sinf(PI2 * 0.8f * t));
}

static void skit_surprised(int eye, float t, float out[P_COUNT])
{
    // Wide eyes, a moment frozen, then calming down.
    open_pose(out);
    if (t >= 0.5f && t < 2.4f) {
        out[P_W] = 134.0f;
        out[P_H] = 170.0f;
        out[P_Y] = -8.0f;
    }
}

static void skit_wink(int eye, float t, float out[P_COUNT])
{
    // The right eye winks while the left one smiles.
    open_pose(out);
    bool winking = t >= 0.9f && t < 1.8f;
    if (winking) {
        out[P_HAPPY] = 0.3f;
        if (eye == 1) {
            out[P_H] = 8.0f;
            out[P_HAPPY] = 0;
        }
    }
    out[P_Y] = winking ? -4.0f : 0;
}

static void skit_angry(int eye, float t, float out[P_COUNT])
{
    // Inner corners down, a fuming tremble.
    out[P_W] = 118.0f;
    out[P_H] = 104.0f;
    out[P_LID_IN] = 0.46f;
    out[P_Y] = 4.0f;
    out[P_X] = t > 0.5f && t < 2.0f ? 3.0f * sinf(PI2 * 9.0f * t) : 0;
}

static void skit_sad(int eye, float t, float out[P_COUNT])
{
    // Outer corners down, looking at the floor.
    out[P_W] = 108.0f;
    out[P_H] = 100.0f;
    out[P_LID_OUT] = 0.44f;
    out[P_Y] = t < 0.6f ? 4.0f : 18.0f;
    out[P_X] = t < 0.6f ? 0 : -8.0f;
}

static void skit_dizzy(int eye, float t, float out[P_COUNT])
{
    // Spinning round in opposite directions, slowing to a stop.
    float r = 24.0f * fminf(1.0f, t / 0.5f) * fminf(1.0f, fmaxf(0, (4.0f - t) / 1.2f));
    float a = PI2 * 1.3f * t;
    out[P_W] = 98.0f;
    out[P_H] = 98.0f;
    out[P_X] = r * cosf(eye == 0 ? a : -a);
    out[P_Y] = r * sinf(eye == 0 ? a : -a);
}

static void skit_cross_eyed(int eye, float t, float out[P_COUNT])
{
    // Both looking at the tip of the nose.
    out[P_W] = 96.0f;
    out[P_H] = 110.0f;
    bool crossed = t >= 0.5f && t < 2.4f;
    out[P_X] = crossed ? (eye == 0 ? 32.0f : -32.0f) : 0;
    out[P_Y] = crossed ? 10.0f : 0;
}

static void skit_eye_roll(int eye, float t, float out[P_COUNT])
{
    // Up and across in an arc, then a flat, unimpressed stare.
    out[P_W] = 112.0f;
    out[P_H] = 110.0f;
    if (t >= 0.5f && t < 1.9f) {
        float a = 3.1415927f * (t - 0.5f) / 1.4f;
        out[P_X] = -32.0f * cosf(a);
        out[P_Y] = -30.0f * sinf(a) - 6.0f;
    } else if (t >= 1.9f) {
        out[P_H] = 86.0f;
        out[P_LID_IN] = out[P_LID_OUT] = 0.3f;
    }
}

static void skit_curious(int eye, float t, float out[P_COUNT])
{
    // One eye big, one narrowed, peering to one side and then the other.
    bool big = (eye == 0) == (t < 2.0f);
    out[P_W] = big ? 122.0f : 100.0f;
    out[P_H] = big ? 150.0f : 96.0f;
    out[P_LID_IN] = big ? 0 : 0.16f;
    out[P_X] = t < 0.4f ? 0 : t < 2.0f ? 26.0f : -26.0f;
    out[P_Y] = t < 0.4f ? 0 : -10.0f;
}

static void skit_love(int eye, float t, float out[P_COUNT])
{
    // Pink, smiling, beating like a heart: two quick pulses a second.
    float beat = fmodf(t, 1.0f);
    float pulse = expf(-40.0f * beat * beat) + 0.7f * expf(-40.0f * (beat - 0.22f) * (beat - 0.22f));
    out[P_W] = 112.0f * (1.0f + 0.12f * pulse);
    out[P_H] = 118.0f * (1.0f + 0.12f * pulse);
    out[P_HAPPY] = 0.36f;
    out[P_Y] = -4.0f;
}

static void skit_scan(int eye, float t, float out[P_COUNT])
{
    // Robot mode: thin slits sweeping from side to side.
    out[P_W] = 128.0f;
    out[P_H] = t < 0.3f || t > 3.4f ? 110.0f : 30.0f;
    out[P_X] = t < 0.5f || t > 3.2f ? 0 : 40.0f * sinf(PI2 * (t - 0.5f) / 1.35f);
}

static void skit_shy(int eye, float t, float out[P_COUNT])
{
    // A smile, looking away and down, a peek back.
    out[P_W] = 106.0f;
    out[P_H] = 104.0f;
    out[P_HAPPY] = 0.26f;
    bool peek = t >= 2.2f && t < 2.8f;
    out[P_X] = t < 0.4f ? 0 : peek ? -6.0f : -30.0f;
    out[P_Y] = t < 0.4f ? 0 : peek ? 2.0f : 20.0f;
}

static void skit_flutter(int eye, float t, float out[P_COUNT])
{
    // A burst of quick blinks, as if something got in.
    open_pose(out);
    bool burst = t >= 0.6f && t < 1.8f;
    if (burst && fmodf(t - 0.6f, 0.3f) < 0.12f) {
        out[P_H] = 10.0f;
    }
    out[P_Y] = burst ? 4.0f : 0;
}

static float smooth01(float x)
{
    x = x < 0 ? 0 : x > 1 ? 1 : x;
    return x * x * (3.0f - 2.0f * x);
}

/* A scrambled `n`, the same on the mirror (eyes.ts). */
static uint32_t hash32(uint32_t n)
{
    n *= 2654435761u;
    n ^= n >> 15;
    n *= 2246822519u;
    n ^= n >> 13;
    return n;
}

static void skit_yawn(int eye, float t, float out[P_COUNT])
{
    // Squeezed into wide slits as the mouth would open, then heavy and slow.
    open_pose(out);
    if (t >= 0.3f && t < 1.5f) {
        out[P_W] = 132.0f;
        out[P_H] = 22.0f;
        out[P_Y] = -10.0f;
    } else if (t >= 1.5f) {
        out[P_H] = t >= 2.6f && t < 2.9f ? 10.0f : t < 2.6f ? 70.0f : 92.0f;
        out[P_LID_IN] = out[P_LID_OUT] = 0.3f;
        out[P_Y] = 6.0f;
    }
}

static void skit_sneeze(int eye, float t, float out[P_COUNT])
{
    // Ah... ah... (wider, higher) CHOO: shut tight, ducking; a shake after.
    open_pose(out);
    if (t >= 0.5f && t < 0.74f) {
        out[P_W] = 118.0f;
        out[P_H] = 146.0f;
        out[P_Y] = -10.0f;
    } else if (t >= 0.74f && t < 0.84f) {
        out[P_H] = 128.0f;
        out[P_Y] = -4.0f;
    } else if (t >= 0.84f && t < 1.18f) {
        out[P_W] = 124.0f;
        out[P_H] = 160.0f;
        out[P_Y] = -16.0f;
    } else if (t >= 1.18f && t < 1.5f) {
        out[P_W] = 134.0f;
        out[P_H] = 8.0f;
        out[P_Y] = 18.0f;
    } else if (t >= 1.5f && t < 2.4f) {
        out[P_X] = 10.0f * sinf(PI2 * 7.0f * t) * expf(-3.0f * (t - 1.5f));
        out[P_H] = 110.0f;
        out[P_LID_OUT] = 0.15f;
    }
}

static void skit_giggle(int eye, float t, float out[P_COUNT])
{
    // Smiling, shaking with laughter.
    out[P_W] = 118.0f;
    out[P_H] = 104.0f;
    out[P_HAPPY] = 0.5f;
    float shake = t > 0.3f ? fabsf(sinf(PI2 * 4.5f * t)) : 0;
    out[P_Y] = -4.0f - 8.0f * shake;
}

static void skit_excited(int eye, float t, float out[P_COUNT])
{
    // Big, sparkling and jumping up and down.
    float pulse = sinf(PI2 * 2.5f * t);
    out[P_W] = 124.0f + 6.0f * pulse;
    out[P_H] = 146.0f + 8.0f * pulse;
    out[P_Y] = -10.0f - 18.0f * fabsf(sinf(PI2 * 1.25f * t));
    out[P_HAPPY] = 0.22f;
}

static void skit_bored(int eye, float t, float out[P_COUNT])
{
    // Half shut, a sigh that sinks them, a slow look away.
    out[P_W] = 112.0f;
    out[P_H] = 72.0f;
    out[P_LID_IN] = out[P_LID_OUT] = 0.42f;
    out[P_Y] = t < 1.2f ? 4.0f : 14.0f;
    out[P_X] = t < 2.5f ? 0 : -24.0f;
}

static void skit_confused(int eye, float t, float out[P_COUNT])
{
    // One eye raised and big, the other low and squinting; then the other way.
    bool raised = (eye == 0) != (t >= 2.0f);
    out[P_W] = raised ? 116.0f : 104.0f;
    out[P_H] = raised ? 140.0f : 92.0f;
    out[P_Y] = raised ? -14.0f : 8.0f;
    out[P_LID_IN] = raised ? 0 : 0.28f;
    out[P_X] = t < 0.3f ? 0 : 10.0f * sinf(PI2 * 0.5f * t);
}

static void skit_scared(int eye, float t, float out[P_COUNT])
{
    // Small and trembling, darting from side to side.
    out[P_W] = 84.0f;
    out[P_H] = 100.0f;
    out[P_LID_OUT] = 0.25f;
    out[P_X] = (t < 0.3f ? 0 : fmodf(t, 1.2f) < 0.6f ? -22.0f : 22.0f) + 2.5f * sinf(PI2 * 13.0f * t);
    out[P_Y] = 6.0f;
}

static void skit_peekaboo(int eye, float t, float out[P_COUNT])
{
    // Hiding, shut... and back, big and delighted.
    if (t < 0.4f) {
        open_pose(out);
    } else if (t < 2.0f) {
        out[P_W] = 124.0f;
        out[P_H] = 6.0f;
    } else {
        out[P_W] = 128.0f;
        out[P_H] = 150.0f;
        out[P_HAPPY] = t > 2.4f ? 0.4f : 0;
        out[P_Y] = -10.0f - 10.0f * fabsf(sinf(PI2 * 1.5f * (t - 2.0f)));
    }
}

static void skit_nod(int eye, float t, float out[P_COUNT])
{
    // Yes, yes, yes.
    open_pose(out);
    out[P_HAPPY] = 0.2f;
    out[P_Y] = t >= 0.4f && t < 2.0f ? 18.0f * fabsf(sinf(3.1415927f * 1.875f * (t - 0.4f))) : 0;
}

static void skit_shake(int eye, float t, float out[P_COUNT])
{
    // No, no, no.
    out[P_W] = 110.0f;
    out[P_H] = 104.0f;
    out[P_LID_IN] = out[P_LID_OUT] = 0.14f;
    out[P_X] = t >= 0.4f && t < 2.0f ? 26.0f * sinf(PI2 * 1.9f * (t - 0.4f)) : 0;
}

static void skit_hiccup(int eye, float t, float out[P_COUNT])
{
    // A jump at every hic.
    open_pose(out);
    if (t >= 0.6f && fmodf(t - 0.6f, 1.2f) < 0.14f) {
        out[P_Y] = -20.0f;
        out[P_W] = 104.0f;
        out[P_H] = 150.0f;
    }
}

static void skit_mischief(int eye, float t, float out[P_COUNT])
{
    // A sly grin, lids down at the inner corners, a look each way.
    out[P_W] = 118.0f;
    out[P_H] = 86.0f;
    out[P_LID_IN] = 0.34f;
    out[P_HAPPY] = 0.3f;
    out[P_X] = t < 0.5f ? 0 : t < 2.2f ? -22.0f : 22.0f;
    out[P_Y] = 4.0f;
}

static void skit_dance(int eye, float t, float out[P_COUNT])
{
    // Bouncing on the beat (120 bpm), swaying a bar each way.
    out[P_W] = 116.0f;
    out[P_H] = 120.0f;
    out[P_HAPPY] = 0.3f;
    out[P_Y] = 4.0f - 18.0f * expf(-8.0f * fmodf(t, 0.5f));
    out[P_X] = 20.0f * sinf(3.1415927f * t);
}

static void skit_sing(int eye, float t, float out[P_COUNT])
{
    // Eyes smiling shut, swaying, bobbing on each note.
    out[P_W] = 114.0f;
    out[P_H] = 100.0f;
    out[P_HAPPY] = 0.45f;
    out[P_X] = 12.0f * sinf(PI2 * 0.6f * t);
    out[P_Y] = -4.0f - (t >= 0.3f && t < 1.6f ? 6.0f * fabsf(sinf(3.1415927f * 4.55f * (t - 0.3f))) : 0);
}

static void skit_purr(int eye, float t, float out[P_COUNT])
{
    // Content, all but shut, breathing slowly.
    float breath = sinf(PI2 * 0.6f * t);
    out[P_W] = 120.0f + 3.0f * breath;
    out[P_H] = 76.0f;
    out[P_HAPPY] = 0.55f;
    out[P_LID_IN] = out[P_LID_OUT] = 0.1f;
    out[P_Y] = 2.0f + 3.0f * breath;
}

static void skit_sigh(int eye, float t, float out[P_COUNT])
{
    // A big breath in, looking up, then sinking with the sigh.
    open_pose(out);
    if (t >= 0.5f && t < 0.8f) {
        out[P_W] = 114.0f;
        out[P_H] = 136.0f;
        out[P_Y] = -12.0f;
    } else if (t >= 0.8f) {
        float d = smooth01((t - 0.8f) / 0.9f);
        out[P_W] = 112.0f;
        out[P_H] = 136.0f - 50.0f * d;
        out[P_LID_IN] = out[P_LID_OUT] = 0.32f * d;
        out[P_Y] = -12.0f + 24.0f * d;
    }
}

static void skit_focus(int eye, float t, float out[P_COUNT])
{
    // Narrowing on something just in front: zoom, lock.
    if (t < 0.4f) {
        open_pose(out);
        return;
    }
    bool zoom = t >= 0.5f && t < 0.9f;
    out[P_W] = zoom ? 96.0f : 106.0f;
    out[P_H] = zoom ? 54.0f : 68.0f;
    out[P_LID_IN] = 0.12f;
    out[P_LID_OUT] = 0.06f;
    out[P_X] = eye == 0 ? 8.0f : -8.0f;
}

static void skit_snore(int eye, float t, float out[P_COUNT])
{
    // Dropping off, then a line that swells with each snore.
    if (t < 1.2f) {
        float d = smooth01(t / 1.2f);
        out[P_W] = 110.0f + 14.0f * d;
        out[P_H] = 124.0f - 114.0f * d;
        out[P_LID_IN] = out[P_LID_OUT] = 0.3f * fminf(1.0f, 2.0f * d);
        out[P_Y] = 16.0f * d;
        return;
    }
    float b = fmodf(t - 1.5f + 2.5f, 2.5f);
    out[P_W] = 124.0f;
    out[P_H] = 8.0f + (b < 0.9f ? 8.0f * sinf(3.1415927f * b / 0.9f) : 0);
    out[P_Y] = 16.0f;
}

static void skit_glitch(int eye, float t, float out[P_COUNT])
{
    // Robot fault: jumping about in random boxes.
    open_pose(out);
    if (t > 0.3f && t < 2.2f) {
        uint32_t h = hash32((uint32_t) (t * 14.0f) + (uint32_t) eye * 7u);
        if (h & 1u) {
            out[P_X] = (float) ((h >> 1) % 60u) - 30.0f;
            out[P_Y] = (float) ((h >> 7) % 40u) - 20.0f;
            out[P_W] = 60.0f + (float) ((h >> 13) % 90u);
            out[P_H] = 20.0f + (float) ((h >> 19) % 140u);
        }
    }
}

static void skit_proud(int eye, float t, float out[P_COUNT])
{
    // Chin up, lids half down, smiling.
    out[P_W] = 116.0f;
    out[P_H] = 96.0f;
    out[P_LID_IN] = out[P_LID_OUT] = 0.22f;
    out[P_HAPPY] = 0.3f;
    out[P_Y] = t < 0.3f ? 0 : -16.0f;
}

static void skit_hot(int eye, float t, float out[P_COUNT])
{
    // Too hot: squinting, drooping, panting.
    float pant = fabsf(sinf(PI2 * 2.2f * t));
    out[P_W] = 120.0f + 4.0f * pant;
    out[P_H] = 70.0f - 6.0f * pant;
    out[P_LID_OUT] = 0.3f;
    out[P_LID_IN] = 0.1f;
    out[P_Y] = 8.0f + 6.0f * pant;
}

static void skit_relieved(int eye, float t, float out[P_COUNT])
{
    // A wide-open breath, then a long, happy breath out.
    if (t < 0.3f) {
        open_pose(out);
    } else if (t < 0.6f) {
        out[P_W] = 116.0f;
        out[P_H] = 136.0f;
        out[P_Y] = -8.0f;
    } else {
        float d = smooth01((t - 0.6f) / 0.8f);
        out[P_W] = 116.0f + 4.0f * d;
        out[P_H] = 136.0f - 36.0f * d;
        out[P_HAPPY] = 0.35f * d;
        out[P_LID_IN] = out[P_LID_OUT] = 0.15f * d;
        out[P_Y] = -8.0f + 14.0f * d;
    }
}

static void skit_tired(int eye, float t, float out[P_COUNT])
{
    // The battery running out: shrinking and sinking as it winds down.
    float d = smooth01((t - 0.3f) / 2.6f);
    out[P_W] = 110.0f - 10.0f * d;
    out[P_H] = 110.0f - 64.0f * d;
    out[P_LID_IN] = out[P_LID_OUT] = 0.4f * d;
    out[P_Y] = 22.0f * d;
}

static void skit_charged(int eye, float t, float out[P_COUNT])
{
    // Plugged in: filling up from the bottom, then a bright smile.
    float d = smooth01((t - 0.2f) / 1.4f);
    out[P_W] = 112.0f;
    out[P_H] = 40.0f + 94.0f * d;
    out[P_Y] = 30.0f - 36.0f * d;
    out[P_HAPPY] = t > 1.8f ? 0.32f : 0;
}

/* Names as play_eyes (board_tools.c) takes them. */
static const skit_t SKITS[] = {
    {"look_around", 0x30D5F0, 4300, true, skit_look_around, {{3000, SOUND_HMM}}},
    {"sleepy", 0x6A8CFF, 6000, false, skit_sleepy, {{300, SOUND_YAWN}, {3400, SOUND_STARTLE}}},
    {"suspicious", 0x30D5F0, 4200, false, skit_suspicious, {{500, SOUND_HMM}}},
    {"happy", 0x40E080, 3500, true, skit_happy, {{200, SOUND_HAPPY}}},
    {"surprised", 0x30D5F0, 3300, false, skit_surprised, {{450, SOUND_GASP}}},
    {"wink", 0x30D5F0, 2800, false, skit_wink, {{850, SOUND_WINK}}},
    {"angry", 0xFF5A30, 3200, false, skit_angry, {{400, SOUND_GRUMBLE}}},
    {"sad", 0x4A7BFF, 4200, true, skit_sad, {{500, SOUND_AWW}}},
    {"dizzy", 0xC070FF, 4000, false, skit_dizzy, {{100, SOUND_BOING}}},
    {"cross_eyed", 0x30D5F0, 3000, false, skit_cross_eyed, {{450, SOUND_BOOP}}},
    {"eye_roll", 0x30D5F0, 3400, false, skit_eye_roll, {{1800, SOUND_PFFT}}},
    {"curious", 0x30D5F0, 4000, true, skit_curious, {{400, SOUND_HMM}, {2000, SOUND_CHIRP}}},
    {"love", 0xFF5AA8, 4000, false, skit_love,
     {{0, SOUND_HEARTBEAT}, {1000, SOUND_HEARTBEAT}, {2000, SOUND_HEARTBEAT}, {3000, SOUND_HEARTBEAT}}},
    {"scan", 0x30F0B0, 3800, false, skit_scan, {{300, SOUND_SCAN}}},
    {"shy", 0xFF8AB0, 3600, false, skit_shy, {{400, SOUND_EEP}}},
    {"flutter", 0x30D5F0, 2600, false, skit_flutter, {{600, SOUND_FLUTTER}}},
    {"yawn", 0x7A9CFF, 4200, false, skit_yawn, {{300, SOUND_YAWN}}},
    {"sneeze", 0x30D5F0, 3400, false, skit_sneeze, {{500, SOUND_SNEEZE}}},
    {"giggle", 0x40E080, 3200, false, skit_giggle, {{300, SOUND_GIGGLE}, {1700, SOUND_GIGGLE}}},
    {"excited", 0xFFD040, 3400, false, skit_excited, {{200, SOUND_EXCITED}, {1700, SOUND_EXCITED}}},
    {"bored", 0x8AA4C0, 4600, true, skit_bored, {{1100, SOUND_SIGH}}},
    {"confused", 0xC0A0FF, 3800, true, skit_confused, {{400, SOUND_CONFUSED}}},
    {"scared", 0xA0C8FF, 3800, false, skit_scared, {{300, SOUND_WHIMPER}}},
    {"peekaboo", 0x40E0C0, 3800, false, skit_peekaboo, {{2000, SOUND_BOO}, {2500, SOUND_GIGGLE}}},
    {"nod", 0x40E080, 2800, true, skit_nod, {{400, SOUND_YES}}},
    {"shake", 0x30D5F0, 2800, false, skit_shake, {{400, SOUND_NOPE}}},
    {"hiccup", 0x30D5F0, 4200, true, skit_hiccup, {{600, SOUND_HICCUP}, {1800, SOUND_HICCUP}, {3000, SOUND_HICCUP}}},
    {"mischief", 0xB060FF, 3800, false, skit_mischief, {{600, SOUND_MISCHIEF}}},
    {"dance", RAINBOW, 4400, false, skit_dance, {{0, SOUND_BEAT}, {2000, SOUND_BEAT}}},
    {"sing", 0x70E0FF, 3200, false, skit_sing, {{300, SOUND_SING}}},
    {"purr", 0xFFB070, 3600, false, skit_purr, {{400, SOUND_PURR}, {2000, SOUND_PURR}}},
    {"sigh", 0x5A8AE0, 3600, false, skit_sigh, {{500, SOUND_SIGH}}},
    {"focus", 0x40FFD0, 3200, false, skit_focus, {{500, SOUND_LOCK}}},
    {"snore", 0x34507A, 6500, false, skit_snore, {{1500, SOUND_SNORE}, {4000, SOUND_SNORE}}},
    {"glitch", 0x30F0B0, 2800, false, skit_glitch, {{300, SOUND_GLITCH}, {1400, SOUND_GLITCH}}},
    {"proud", 0xFFD040, 3200, true, skit_proud, {{300, SOUND_TADA}}},
    {"hot", 0xFF4A30, 4200, false, skit_hot, {{300, SOUND_PANT}, {2000, SOUND_PANT}}},
    {"relieved", 0x60E0FF, 3200, true, skit_relieved, {{400, SOUND_PHEW}}},
    {"tired", 0xFFA030, 4200, false, skit_tired, {{300, SOUND_DRAIN}}},
    {"charged", 0x40E080, 3200, false, skit_charged, {{200, SOUND_CHARGE}}},
};

#define SKIT_COUNT (sizeof(SKITS) / sizeof(SKITS[0]))

/* The eyes face between conversations: open, looking where s_look_* says,
 * sinking into a flat, slowly breathing line as it dozes off. */
static void ambient_pose(float t, float out[P_COUNT])
{
    float d = s_doze;
    out[P_W] = 110.0f + 8.0f * d;
    out[P_H] = 124.0f - 108.0f * d + 3.0f * d * sinf(PI2 * t / 4.0f);
    out[P_X] = s_look_x * (1.0f - d);
    out[P_Y] = s_look_y * (1.0f - d) + 16.0f * d;
    out[P_LID_IN] = out[P_LID_OUT] = 0.3f * fminf(1.0f, 2.0f * d);
    // Awake, it wears its mood: heavy lids when tired, a smile when happy.
    float droop, smile;
    pet_pose(&droop, &smile);
    out[P_LID_IN] += droop * (1.0f - d);
    out[P_LID_OUT] += droop * (1.0f - d);
    out[P_HAPPY] = smile * (1.0f - d);
}

/** What eye `eye` should look like `t` seconds into `state`. The left eye is
 * screen 0, so its inner corner is on the right. */
static void expression(voice_state_t state, int eye, float t, float out[P_COUNT])
{
    float level = s_level;
    memset(out, 0, sizeof(float) * P_COUNT);
    if (state == VOICE_IDLE && s_skit != NULL) {
        s_skit->fn(eye, t, out);
        return;
    }
    switch (state) {
    case VOICE_LISTENING:
        // Wide open and attentive, a touch bigger the louder you are.
        out[P_W] = 112.0f * (1.0f + 0.10f * level);
        out[P_H] = 136.0f * (1.0f + 0.10f * level);
        out[P_X] = s_gaze_x;
        out[P_Y] = s_gaze_y;
        break;
    case VOICE_THINKING:
        // Looking up, slowly from side to side; one eye squinting.
        out[P_W] = 108.0f;
        out[P_H] = eye == 0 ? 92.0f : 104.0f;
        out[P_X] = 26.0f * sinf(PI2 * t / 3.2f);
        out[P_Y] = -24.0f;
        out[P_LID_IN] = out[P_LID_OUT] = eye == 0 ? 0.18f : 0.0f;
        break;
    case VOICE_SPEAKING:
        // Smiling, bobbing up and stretching with the voice.
        out[P_W] = 118.0f;
        out[P_H] = 118.0f * (1.0f + 0.10f * level);
        out[P_X] = s_gaze_x * 0.5f;
        out[P_Y] = -6.0f - 12.0f * level;
        out[P_HAPPY] = 0.28f + 0.18f * level;
        break;
    case VOICE_ERROR:
        // Sad, with a shake of the head that dies away.
        out[P_W] = 110.0f;
        out[P_H] = 104.0f;
        out[P_X] = 12.0f * sinf(PI2 * 6.0f * t) * expf(-2.5f * t);
        out[P_Y] = 6.0f;
        out[P_LID_IN] = 0.05f;
        out[P_LID_OUT] = 0.38f;
        break;
    default:
        if (s_ambient[eye]) {
            ambient_pose(t, out);
            break;
        }
        // Shut: a flat line.
        out[P_W] = 124.0f;
        out[P_H] = 6.0f;
        break;
    }
}

/** In steps of at most SPRING_STEP_S: a late frame would otherwise throw a
 * stiff spring off to infinity. */
static void spring_step(spring_t *s, float target, float k, float zeta, float dt)
{
    int n = (int) ceilf(dt / SPRING_STEP_S);
    float h = dt / (float) n;
    float c = 2.0f * zeta * sqrtf(k);
    for (int i = 0; i < n; i++) {
        s->vel += (k * (target - s->v) - c * s->vel) * h;
        s->v += s->vel * h;
    }
}

static void blink_step(uint32_t now)
{
    bool skit = s_state == VOICE_IDLE && s_skit != NULL;
    // The eyes face blinks too, until it dozes.
    bool awake = s_state == VOICE_IDLE && !skit && any_ambient() && s_doze < 0.3f;
    if (skit ? !s_skit->blinks : !awake && (s_state == VOICE_IDLE || s_state == VOICE_ERROR)) {
        s_blinking = false;
        s_blink = 1.0f;
        return;
    }
    if (!s_blinking && (int32_t) (now - s_next_blink_ms) >= 0) {
        s_blinking = true;
        s_blink_ms = now;
    }
    if (!s_blinking) {
        s_blink = 1.0f;
        return;
    }
    uint32_t t = now - s_blink_ms;
    if (t < BLINK_DOWN_MS) {
        s_blink = 1.0f - (1.0f - BLINK_SHUT) * (float) t / BLINK_DOWN_MS;
    } else if (t < BLINK_DOWN_MS + BLINK_HOLD_MS) {
        s_blink = BLINK_SHUT;
    } else if (t < BLINK_DOWN_MS + BLINK_HOLD_MS + BLINK_UP_MS) {
        s_blink = BLINK_SHUT + (1.0f - BLINK_SHUT) * (float) (t - BLINK_DOWN_MS - BLINK_HOLD_MS) / BLINK_UP_MS;
    } else {
        s_blink = 1.0f;
        s_blinking = false;
        if (s_blink_again) {
            s_blink_again = false;
            s_next_blink_ms = now + 90;
        } else {
            // Thinking eyes blink less.
            s_next_blink_ms = now + (s_state == VOICE_THINKING ? lv_rand(3500, 7000) : lv_rand(2200, 5500));
            s_blink_again = lv_rand(0, 4) == 0;
        }
    }
}

/** Now and then a quick glance somewhere else, or back to you. */
static void glance_step(uint32_t now)
{
    if (s_state != VOICE_LISTENING && s_state != VOICE_SPEAKING) {
        s_gaze_x = s_gaze_y = 0;
        return;
    }
    if ((int32_t) (now - s_next_glance_ms) < 0) {
        return;
    }
    if (lv_rand(0, 9) < 3) {
        s_gaze_x = s_gaze_y = 0;
    } else {
        s_gaze_x = (float) lv_rand(0, 24) - 12.0f;
        s_gaze_y = (float) lv_rand(0, 16) - 8.0f;
    }
    s_next_glance_ms = now + lv_rand(900, 2800);
}

/* The pointer moved within `ms`. A move stamped after `now` counts too. */
static bool pointer_within(uint32_t now, uint32_t ms)
{
    uint32_t moved = s_ptr_ms;
    return moved != 0 && (int32_t) (now - moved) < (int32_t) ms;
}

/** The eyes face: look at the pointer while it moves, about the room when
 * it doesn't, doze off when it has been still a long while, wake with a
 * start when it moves again. */
static void ambient_step(uint32_t now)
{
    if (!any_ambient()) {
        s_doze = 0;
        return;
    }
    if (pointer_within(now, POINTER_FRESH_MS)) {
        s_look_x = LOOK_X * (float) s_ptr_x / 1000.0f;
        s_look_y = LOOK_Y_BIAS + LOOK_Y * (float) s_ptr_y / 1000.0f;
    } else if ((int32_t) (now - s_next_look_ms) >= 0) {
        if (lv_rand(0, 9) < 4) {
            s_look_x = 0;
            s_look_y = LOOK_Y_BIAS / 2;
        } else {
            s_look_x = (float) lv_rand(0, 48) - 24.0f;
            s_look_y = (float) lv_rand(0, 28) - 16.0f;
        }
        s_next_look_ms = now + lv_rand(1500, 4500);
    }

    if (s_state != VOICE_IDLE) {
        s_awake_ms = now;
    }
    float doze = 0;
    if (s_ptr_ms != 0) {
        uint32_t since = s_ptr_ms;
        if ((int32_t) (s_awake_ms - since) > 0) {
            since = s_awake_ms;
        }
        int32_t still = (int32_t) (now - since);
        if (still > DOZE_AFTER_MS) {
            doze = fminf(1.0f, (float) (still - DOZE_AFTER_MS) / DOZE_MS);
        }
    }
    if (doze == 0 && s_doze > 0.3f && s_state == VOICE_IDLE && s_skit == NULL) {
        // Woken up: eyes wide for a moment, "huh?!".
        playback_sound(SOUND_STARTLE);
        for (int i = 0; i < BOARD_LCD_COUNT; i++) {
            if (s_ambient[i]) {
                s_eyes[i].p[P_H].vel += 420.0f;
            }
        }
        s_next_blink_ms = now + lv_rand(300, 700);
    }
    s_doze = doze;
    if (doze < 1.0f) {
        s_snores = 0;
        s_next_snore_ms = now + SNORE_EVERY_MS / 2;
    } else if (s_state == VOICE_IDLE && s_skit == NULL && s_snores < SNORES
               && (int32_t) (now - s_next_snore_ms) >= 0) {
        playback_sound(SOUND_SNORE);
        s_snores++;
        s_next_snore_ms = now + SNORE_EVERY_MS;
    }
}

static void color_step(void)
{
    lv_color_t target = lv_color_hex(state_color(s_state));
    const float to[3] = {target.red, target.green, target.blue};
    for (int c = 0; c < 3; c++) {
        s_rgb[c] += (to[c] - s_rgb[c]) * COLOR_RATE;
    }
    s_color = lv_color_make((uint8_t) (s_rgb[0] + 0.5f), (uint8_t) (s_rgb[1] + 0.5f), (uint8_t) (s_rgb[2] + 0.5f));
}

static void compute_geom(int eye, geom_t *g)
{
    const spring_t *p = s_eyes[eye].p;
    float w = fmaxf(p[P_W].v, 4.0f);
    float h = fmaxf(p[P_H].v * s_blink, 3.0f);
    float cx = BOARD_LCD_H_RES / 2.0f + p[P_X].v;
    float cy = BOARD_LCD_V_RES / 2.0f + p[P_Y].v;
    g->x0 = lroundf(cx - w / 2);
    g->x1 = lroundf(cx + w / 2);
    g->y0 = lroundf(cy - h / 2);
    g->y1 = lroundf(cy + h / 2);
    g->radius = lroundf(fminf(w, h) * RADIUS_SHARE);
    int32_t lid_in = lroundf(fmaxf(p[P_LID_IN].v, 0) * h);
    int32_t lid_out = lroundf(fmaxf(p[P_LID_OUT].v, 0) * h);
    g->lid_left = eye == 0 ? lid_out : lid_in;
    g->lid_right = eye == 0 ? lid_in : lid_out;
    float happy = p[P_HAPPY].v;
    g->happy_top = happy > 0.02f ? lroundf(cy + h / 2 - happy * h * 0.8f) : 0;
}

static void hide_eye(eye_t *e)
{
    e->shown = false;
    lv_obj_add_flag(e->stage, LV_OBJ_FLAG_HIDDEN);
    lv_obj_remove_local_style_prop(e->screen, LV_STYLE_OPA_LAYERED, LV_PART_MAIN);
}

/* Settle s_open once something changed; stop drawing when no eye is up. */
static void eyes_settle(void)
{
    bool talking = s_state != VOICE_IDLE || s_skit != NULL;
    bool any = false;
    bool others = false;
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        any = any || s_eyes[i].shown;
        others = others || (s_eyes[i].shown && !s_ambient[i]);
    }
    s_open = talking || others;
    if (!s_open) {
        s_closing = NULL;
    }
    if (!any) {
        lv_timer_pause(s_timer);
    }
}

/* The eyes off the screens that don't show the eyes face. */
static void hide_eyes(void)
{
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        if (s_eyes[i].shown && !s_ambient[i]) {
            hide_eye(&s_eyes[i]);
        }
    }
    eyes_settle();
}

static void frame(lv_timer_t *timer)
{
    uint32_t now = lv_tick_get();
    float dt = (float) (now - s_last_ms) / 1000.0f;
    dt = dt < 0.001f ? 0.001f : dt > 0.08f ? 0.08f : dt;
    s_last_ms = now;
    if (s_skit != NULL && now - s_skit_ms >= s_skit->ms) {
        // Scene over: close like at the end of a conversation.
        s_closing = s_skit;
        s_skit = NULL;
        s_state_ms = now;
    }
    float t = (float) ((s_skit != NULL ? now - s_skit_ms : now - s_state_ms)) / 1000.0f;
    while (s_skit != NULL && s_cue < MAX_CUES && s_skit->cues[s_cue].sound != SOUND_WAKE
           && now - s_skit_ms >= s_skit->cues[s_cue].ms) {
        playback_sound(s_skit->cues[s_cue++].sound);
    }

    s_level += (s_level_in - s_level) * fminf(1.0f, dt * 12.0f);
    ambient_step(now);
    blink_step(now);
    glance_step(now);
    lv_color_t was = s_color;
    color_step();
    bool recolored = !lv_color_eq(was, s_color);

    bool talking = s_state != VOICE_IDLE || s_skit != NULL;
    bool hid = false;
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        eye_t *e = &s_eyes[i];
        if (!e->shown) {
            continue;
        }
        float target[P_COUNT];
        expression(s_state, i, t, target);
        for (int k = 0; k < P_COUNT; k++) {
            bool gaze = k == P_X || k == P_Y;
            spring_step(&e->p[k], target[k], gaze ? GAZE_K : SHAPE_K, gaze ? GAZE_ZETA : SHAPE_ZETA, dt);
        }
        // Shut and not wanted: the watch face comes back on this screen.
        bool shut = e->p[P_H].v < CLOSED_H;
        if (!talking && !s_ambient[i] && shut && now - s_state_ms >= CLOSE_MIN_MS && now - e->shown_ms >= CLOSE_MIN_MS) {
            hide_eye(e);
            hid = true;
        }
    }
    if (hid || (!talking && s_open)) {
        eyes_settle();
        if (lv_timer_get_paused(s_timer)) {
            return;
        }
    }

    // Repaint only the box the eye was in and the one it's in now.
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        eye_t *e = &s_eyes[i];
        if (!e->shown) {
            continue;
        }
        geom_t g;
        compute_geom(i, &g);
        if (e->drawn && !recolored && memcmp(&g, &e->geom, sizeof(g)) == 0) {
            continue;
        }
        lv_area_t area = {g.x0, g.y0, g.x1, g.y1};
        if (e->drawn) {
            area.x1 = LV_MIN(area.x1, e->geom.x0);
            area.y1 = LV_MIN(area.y1, e->geom.y0);
            area.x2 = LV_MAX(area.x2, e->geom.x1);
            area.y2 = LV_MAX(area.y2, e->geom.y1);
        }
        e->geom = g;
        e->drawn = true;
        lv_obj_invalidate_area(e->stage, &area);
    }
}

static void draw_eye(lv_event_t *ev)
{
    const eye_t *e = lv_event_get_user_data(ev);
    if (!e->drawn) {
        return;
    }
    lv_layer_t *layer = lv_event_get_layer(ev);
    const geom_t *g = &e->geom;

    lv_draw_rect_dsc_t rect;
    lv_draw_rect_dsc_init(&rect);
    rect.bg_color = s_color;
    rect.bg_opa = LV_OPA_COVER;
    rect.radius = g->radius;
    lv_area_t a = {g->x0, g->y0, g->x1, g->y1};
    lv_draw_rect(layer, &rect, &a);

    if (g->lid_left > 0 || g->lid_right > 0) {
        // The top lid: a black quad over the eye, slanted for a mood.
        lv_draw_triangle_dsc_t lid;
        lv_draw_triangle_dsc_init(&lid);
        lid.color = lv_color_black();
        lid.opa = LV_OPA_COVER;
        int32_t l = g->x0 - 2, r = g->x1 + 2, top = g->y0 - 2;
        lid.p[0] = (lv_point_precise_t) {l, top};
        lid.p[1] = (lv_point_precise_t) {r, top};
        lid.p[2] = (lv_point_precise_t) {r, g->y0 + g->lid_right};
        lv_draw_triangle(layer, &lid);
        lid.p[1] = (lv_point_precise_t) {r, g->y0 + g->lid_right};
        lid.p[2] = (lv_point_precise_t) {l, g->y0 + g->lid_left};
        lv_draw_triangle(layer, &lid);
    }

    if (g->happy_top > 0) {
        // The smile: a big black circle rising under the eye leaves a crescent.
        int32_t d = (g->x1 - g->x0) * SMILE_SHARE / 10;
        int32_t cx = (g->x0 + g->x1) / 2;
        lv_draw_rect_dsc_t arch;
        lv_draw_rect_dsc_init(&arch);
        arch.bg_color = lv_color_black();
        arch.bg_opa = LV_OPA_COVER;
        arch.radius = LV_RADIUS_CIRCLE;
        lv_area_t b = {cx - d / 2, g->happy_top, cx + d / 2, g->happy_top + d};
        lv_draw_rect(layer, &arch, &b);
    }
}

static void open_eyes(uint32_t now);
static void idle_check(lv_timer_t *timer);

void ui_eyes_create(lv_display_t *const displays[BOARD_LCD_COUNT])
{
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        eye_t *e = &s_eyes[i];
        e->screen = lv_display_get_screen_active(displays[i]);
        e->stage = lv_obj_create(lv_display_get_layer_top(displays[i]));
        lv_obj_remove_style_all(e->stage);
        lv_obj_set_size(e->stage, BOARD_LCD_H_RES, BOARD_LCD_V_RES);
        lv_obj_set_pos(e->stage, 0, 0);
        lv_obj_set_style_bg_color(e->stage, lv_color_black(), 0);
        lv_obj_set_style_bg_opa(e->stage, LV_OPA_COVER, 0);
        lv_obj_remove_flag(e->stage, LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
        lv_obj_add_flag(e->stage, LV_OBJ_FLAG_HIDDEN);
        lv_obj_add_event_cb(e->stage, draw_eye, LV_EVENT_DRAW_MAIN_END, e);
        // Under the voice ring, which may go round an eyes face.
        lv_obj_move_to_index(e->stage, 0);
    }
    s_timer = lv_timer_create(frame, FRAME_MS, NULL);
    lv_timer_pause(s_timer);
    // LVGL's generator starts from the same seed on every boot.
    lv_rand_set_seed(esp_random() | 1);
    s_idle_timer = lv_timer_create(idle_check, IDLE_CHECK_MS, NULL);
}

/* Put eye `e` up, shut, so it opens on its springs. When it's the first,
 * start drawing in the colour of the moment. */
static void show_eye(eye_t *e, uint32_t now)
{
    if (!s_eyes[0].shown && !s_eyes[1].shown) {
        lv_color_t c = lv_color_hex(state_color(s_state));
        s_rgb[0] = c.red;
        s_rgb[1] = c.green;
        s_rgb[2] = c.blue;
        s_color = c;
        s_last_ms = now;
        lv_timer_resume(s_timer);
    }
    float shut[P_COUNT] = {0};
    shut[P_W] = 124.0f;
    shut[P_H] = 6.0f;
    for (int k = 0; k < P_COUNT; k++) {
        e->p[k] = (spring_t) {.v = shut[k]};
    }
    e->drawn = false;
    e->shown = true;
    e->shown_ms = now;
    // Not LV_OBJ_FLAG_HIDDEN: un-hiding a screen makes LVGL mark its
    // (missing) parent's layout dirty and crash. Fully transparent, the
    // screen and the faces on it aren't drawn at all.
    lv_obj_set_style_opa_layered(e->screen, LV_OPA_TRANSP, LV_PART_MAIN);
    lv_obj_remove_flag(e->stage, LV_OBJ_FLAG_HIDDEN);
}

/* Both eyes up for a conversation or a scene; one already open on an eyes
 * face carries on from where it is. */
static void open_eyes(uint32_t now)
{
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        if (!s_eyes[i].shown) {
            show_eye(&s_eyes[i], now);
        }
    }
    s_level = s_level_in = 0;
    s_gaze_x = s_gaze_y = 0;
    s_open = true;
}

static bool both_shown(void)
{
    return s_eyes[0].shown && s_eyes[1].shown;
}

void ui_eyes_show(voice_state_t state)
{
    if (state == s_state) {
        return;
    }
    uint32_t now = lv_tick_get();
    voice_state_t was = s_state;
    s_state = state;
    s_state_ms = now;
    if (state == VOICE_IDLE) {
        return;
    }
    // A conversation takes over a scene from where the eyes are.
    s_skit = NULL;
    s_closing = NULL;
    if (!s_open || !both_shown()) {
        open_eyes(now);
    } else if (state == VOICE_LISTENING && was != VOICE_IDLE) {
        // Listening again (a follow-up): perk up.
        for (int i = 0; i < BOARD_LCD_COUNT; i++) {
            s_eyes[i].p[P_H].vel += 320.0f;
        }
    }
    s_blinking = false;
    s_blink_again = false;
    s_blink = 1.0f;
    s_next_blink_ms = now + lv_rand(900, 2200);
    s_next_glance_ms = now + lv_rand(700, 1500);
}

void ui_eyes_set_level(float level)
{
    s_level_in = level < 0 ? 0 : level > 1 ? 1 : level;
}

static void play(const skit_t *skit)
{
    uint32_t now = lv_tick_get();
    s_skit = s_last_skit = skit;
    s_closing = NULL;
    s_skit_ms = now;
    s_cue = 0;
    if (!s_open || !both_shown()) {
        open_eyes(now);
    }
    s_blinking = false;
    s_blink_again = false;
    s_blink = 1.0f;
    s_next_blink_ms = now + lv_rand(900, 2200);
}

/* Sooner when the pet is lively. */
static uint32_t idle_gap_ms(void)
{
    float pace = pet_idle_pace();
    return (uint32_t) (pace * IDLE_MIN_S * 1000) + lv_rand(0, (uint32_t) (pace * (IDLE_MAX_S - IDLE_MIN_S) * 1000));
}

static const skit_t *find_skit(const char *name)
{
    for (size_t i = 0; i < SKIT_COUNT; i++) {
        if (strcmp(SKITS[i].name, name) == 0) {
            return &SKITS[i];
        }
    }
    return NULL;
}

/** Once a second: a scene when it's been quiet long enough. Anything on the
 * screens (a conversation, the ring, a scene) starts the wait over. */
static void idle_check(lv_timer_t *timer)
{
    uint32_t now = lv_tick_get();
    // Not over an eyes face that has dozed off, nor with nobody there.
    if (!s_idle_on || s_busy || s_open || s_state != VOICE_IDLE || s_doze > 0 || pet_quiet()) {
        s_next_idle_ms = now + idle_gap_ms();
        return;
    }
    if ((int32_t) (now - s_next_idle_ms) < 0) {
        return;
    }
    // One that suits the mood, else any; not the last one again.
    const char *name = pet_idle_scene();
    const skit_t *skit = name != NULL ? find_skit(name) : NULL;
    if (skit == NULL || skit == s_last_skit) {
        uint32_t i = lv_rand(0, SKIT_COUNT - 2);
        if (s_last_skit != NULL && &SKITS[i] >= s_last_skit) {
            i++;
        }
        skit = &SKITS[i];
    }
    play(skit);
}

void ui_eyes_set_idle(bool on)
{
    s_idle_on = on;
    s_next_idle_ms = lv_tick_get() + idle_gap_ms();
    if (!on && s_skit != NULL) {
        s_closing = s_skit;
        s_skit = NULL;
        s_state_ms = lv_tick_get();
    }
}

void ui_eyes_set_busy(bool busy)
{
    s_busy = busy;
    if (busy && s_open && s_state == VOICE_IDLE) {
        // The ring goes over the watch face: no eyes left on top of it (but
        // an eyes face keeps its own, under the ring).
        s_skit = NULL;
        hide_eyes();
    }
}

void ui_eyes_set_ambient(int screen, bool on)
{
    if (screen < 0 || screen >= BOARD_LCD_COUNT || s_ambient[screen] == on) {
        return;
    }
    s_ambient[screen] = on;
    // Off: the eye closes in frame(), unless a conversation holds it open.
    if (!on) {
        return;
    }
    uint32_t now = lv_tick_get();
    s_awake_ms = now;
    s_doze = 0;
    s_next_look_ms = now + lv_rand(800, 2000);
    if (!s_eyes[screen].shown) {
        show_eye(&s_eyes[screen], now);
    }
}

void ui_eyes_set_gaze(float x, float y)
{
    x = x < -1.0f ? -1.0f : x > 1.0f ? 1.0f : x;
    y = y < -1.0f ? -1.0f : y > 1.0f ? 1.0f : y;
    s_ptr_x = (int32_t) (x * 1000.0f);
    s_ptr_y = (int32_t) (y * 1000.0f);
    s_ptr_ms = lv_tick_get() | 1;
}

bool ui_eyes_play(const char *name)
{
    if (s_busy || s_state != VOICE_IDLE) {
        return false;
    }
    if (name == NULL) {
        uint32_t i = lv_rand(0, SKIT_COUNT - 1);
        play(&SKITS[i]);
        return true;
    }
    const skit_t *skit = find_skit(name);
    if (skit != NULL) {
        play(skit);
    }
    return skit != NULL;
}

int ui_eyes_animations(const char **names, int max)
{
    int n = 0;
    for (size_t i = 0; i < SKIT_COUNT && n < max; i++) {
        names[n++] = SKITS[i].name;
    }
    return n;
}
