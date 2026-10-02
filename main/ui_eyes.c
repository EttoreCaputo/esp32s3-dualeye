#include "ui_eyes.h"

#include <math.h>
#include <string.h>

#include "esp_random.h"

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
} eye_t;

static eye_t s_eyes[BOARD_LCD_COUNT];
static lv_timer_t *s_timer;
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

/* A scene the eyes play on their own while idle: what each eye looks like
 * `t` seconds in, for `ms`, in `color`. */
typedef struct {
    const char *name;
    uint32_t color;
    uint32_t ms;
    bool blinks;
    void (*fn)(int eye, float t, float out[P_COUNT]);
} skit_t;

static const skit_t *s_skit;
/* The scene that just ended, its colour kept while the eyes close. */
static const skit_t *s_closing;
static const skit_t *s_last_skit;
static uint32_t s_skit_ms;
static lv_timer_t *s_idle_timer;
static bool s_idle_on;
static bool s_busy;
static uint32_t s_next_idle_ms;

static uint32_t state_color(voice_state_t state)
{
    const skit_t *skit = s_skit != NULL ? s_skit : s_closing;
    if (skit != NULL && state == VOICE_IDLE) {
        return skit->color;
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

/* Names as play_eyes (board_tools.c) takes them. */
static const skit_t SKITS[] = {
    {"look_around", 0x30D5F0, 4300, true, skit_look_around},
    {"sleepy", 0x6A8CFF, 6000, false, skit_sleepy},
    {"suspicious", 0x30D5F0, 4200, false, skit_suspicious},
    {"happy", 0x40E080, 3500, true, skit_happy},
    {"surprised", 0x30D5F0, 3300, false, skit_surprised},
    {"wink", 0x30D5F0, 2800, false, skit_wink},
    {"angry", 0xFF5A30, 3200, false, skit_angry},
    {"sad", 0x4A7BFF, 4200, true, skit_sad},
    {"dizzy", 0xC070FF, 4000, false, skit_dizzy},
    {"cross_eyed", 0x30D5F0, 3000, false, skit_cross_eyed},
    {"eye_roll", 0x30D5F0, 3400, false, skit_eye_roll},
    {"curious", 0x30D5F0, 4000, true, skit_curious},
    {"love", 0xFF5AA8, 4000, false, skit_love},
    {"scan", 0x30F0B0, 3800, false, skit_scan},
    {"shy", 0xFF8AB0, 3600, false, skit_shy},
    {"flutter", 0x30D5F0, 2600, false, skit_flutter},
};

#define SKIT_COUNT (sizeof(SKITS) / sizeof(SKITS[0]))

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
    if (skit ? !s_skit->blinks : s_state == VOICE_IDLE || s_state == VOICE_ERROR) {
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

static void hide_eyes(void)
{
    s_open = false;
    s_closing = NULL;
    lv_timer_pause(s_timer);
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        lv_obj_add_flag(s_eyes[i].stage, LV_OBJ_FLAG_HIDDEN);
        lv_obj_remove_local_style_prop(s_eyes[i].screen, LV_STYLE_OPA_LAYERED, LV_PART_MAIN);
    }
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

    s_level += (s_level_in - s_level) * fminf(1.0f, dt * 12.0f);
    blink_step(now);
    glance_step(now);
    lv_color_t was = s_color;
    color_step();
    bool recolored = !lv_color_eq(was, s_color);

    bool shut = true;
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        eye_t *e = &s_eyes[i];
        float target[P_COUNT];
        expression(s_state, i, t, target);
        for (int k = 0; k < P_COUNT; k++) {
            bool gaze = k == P_X || k == P_Y;
            spring_step(&e->p[k], target[k], gaze ? GAZE_K : SHAPE_K, gaze ? GAZE_ZETA : SHAPE_ZETA, dt);
        }
        shut = shut && e->p[P_H].v < CLOSED_H;
    }
    if (s_state == VOICE_IDLE && s_skit == NULL && shut && now - s_state_ms >= CLOSE_MIN_MS) {
        hide_eyes();
        return;
    }

    // Repaint only the box the eye was in and the one it's in now.
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        eye_t *e = &s_eyes[i];
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
    }
    s_timer = lv_timer_create(frame, FRAME_MS, NULL);
    lv_timer_pause(s_timer);
    // LVGL's generator starts from the same seed on every boot.
    lv_rand_set_seed(esp_random() | 1);
    s_idle_timer = lv_timer_create(idle_check, IDLE_CHECK_MS, NULL);
}

static void open_eyes(uint32_t now)
{
    lv_color_t c = lv_color_hex(state_color(s_state));
    s_rgb[0] = c.red;
    s_rgb[1] = c.green;
    s_rgb[2] = c.blue;
    s_color = c;
    float shut[P_COUNT];
    expression(VOICE_IDLE, 0, 0, shut);
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        eye_t *e = &s_eyes[i];
        for (int k = 0; k < P_COUNT; k++) {
            e->p[k] = (spring_t) {.v = shut[k]};
        }
        e->drawn = false;
        // Not LV_OBJ_FLAG_HIDDEN: un-hiding a screen makes LVGL mark its
        // (missing) parent's layout dirty and crash. Fully transparent, the
        // screen and the faces on it aren't drawn at all.
        lv_obj_set_style_opa_layered(e->screen, LV_OPA_TRANSP, LV_PART_MAIN);
        lv_obj_remove_flag(e->stage, LV_OBJ_FLAG_HIDDEN);
    }
    s_level = s_level_in = 0;
    s_gaze_x = s_gaze_y = 0;
    s_last_ms = now;
    s_open = true;
    lv_timer_resume(s_timer);
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
    if (!s_open) {
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
    if (!s_open) {
        open_eyes(now);
    }
    s_blinking = false;
    s_blink_again = false;
    s_blink = 1.0f;
    s_next_blink_ms = now + lv_rand(900, 2200);
}

static uint32_t idle_gap_ms(void)
{
    return lv_rand(IDLE_MIN_S * 1000, IDLE_MAX_S * 1000);
}

/** Once a second: a scene when it's been quiet long enough. Anything on the
 * screens (a conversation, the ring, a scene) starts the wait over. */
static void idle_check(lv_timer_t *timer)
{
    uint32_t now = lv_tick_get();
    if (!s_idle_on || s_busy || s_open || s_state != VOICE_IDLE) {
        s_next_idle_ms = now + idle_gap_ms();
        return;
    }
    if ((int32_t) (now - s_next_idle_ms) < 0) {
        return;
    }
    // Any but the last one.
    uint32_t i = lv_rand(0, SKIT_COUNT - 2);
    if (s_last_skit != NULL && &SKITS[i] >= s_last_skit) {
        i++;
    }
    play(&SKITS[i]);
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
        // The ring goes over the watch face: no eyes left on top of it.
        s_skit = NULL;
        hide_eyes();
    }
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
    for (size_t i = 0; i < SKIT_COUNT; i++) {
        if (strcmp(SKITS[i].name, name) == 0) {
            play(&SKITS[i]);
            return true;
        }
    }
    return false;
}

int ui_eyes_animations(const char **names, int max)
{
    int n = 0;
    for (size_t i = 0; i < SKIT_COUNT && n < max; i++) {
        names[n++] = SKITS[i].name;
    }
    return n;
}
