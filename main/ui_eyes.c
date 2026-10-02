#include "ui_eyes.h"

#include <math.h>
#include <string.h>

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

static uint32_t state_color(voice_state_t state)
{
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

/** What eye `eye` should look like `t` seconds into `state`. The left eye is
 * screen 0, so its inner corner is on the right. */
static void expression(voice_state_t state, int eye, float t, float out[P_COUNT])
{
    float level = s_level;
    memset(out, 0, sizeof(float) * P_COUNT);
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
    if (s_state == VOICE_IDLE || s_state == VOICE_ERROR) {
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
    float t = (float) (now - s_state_ms) / 1000.0f;

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
    if (s_state == VOICE_IDLE && shut && now - s_state_ms >= CLOSE_MIN_MS) {
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
