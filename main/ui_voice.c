#include "ui_voice.h"

#include "ui_eyes.h"

#define RING_WIDTH 8
#define SPINNER_ARC_DEG 70
#define SPINNER_PERIOD_MS 1200

#define LISTENING_COLOR 0x30D5F0
#define THINKING_COLOR 0xFFB020
#define SPEAKING_COLOR 0x40E080
#define ERROR_COLOR 0xFF4040
#define LEVEL_COLOR 0xE0FAFF
/* The level arc: centred at the top (LVGL: 0 deg is 3 o'clock, clockwise),
 * up to this span (kept small: LVGL redraws the bounding box of what
 * changes), in this many steps so a steady voice doesn't redraw. */
#define LEVEL_CENTER_DEG 270
#define LEVEL_MAX_DEG 100
#define LEVEL_STEPS 10

typedef struct {
    /* Full ring: listening, speaking and error. Drawn once, so it costs one redraw. */
    lv_obj_t *ring;
    /* Turning arc: thinking. LVGL redraws only the arc's old and new sectors. */
    lv_obj_t *spinner;
    /* Over the ring while listening (mic) and speaking (speaker). LVGL redraws only the sectors that change. */
    lv_obj_t *level;
} ui_voice_t;

static ui_voice_t s_voice[BOARD_LCD_COUNT];
static int s_level_step;
static bool s_eyes = true;
static voice_state_t s_state = VOICE_IDLE;

static void style_arc(lv_obj_t *arc)
{
    lv_obj_remove_style_all(arc);
    lv_obj_set_size(arc, BOARD_LCD_H_RES, BOARD_LCD_V_RES);
    lv_obj_center(arc);
    lv_obj_remove_flag(arc, LV_OBJ_FLAG_CLICKABLE);
    lv_obj_set_style_arc_width(arc, RING_WIDTH, LV_PART_INDICATOR);
    lv_obj_set_style_arc_rounded(arc, true, LV_PART_INDICATOR);
    lv_obj_set_style_arc_opa(arc, LV_OPA_TRANSP, LV_PART_MAIN);
    lv_obj_add_flag(arc, LV_OBJ_FLAG_HIDDEN);
}

void ui_voice_create(lv_display_t *const displays[BOARD_LCD_COUNT])
{
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        lv_obj_t *top = lv_display_get_layer_top(displays[i]);
        ui_voice_t *v = &s_voice[i];

        v->ring = lv_arc_create(top);
        style_arc(v->ring);
        lv_arc_set_bg_angles(v->ring, 0, 360);
        lv_arc_set_angles(v->ring, 0, 360);

        v->level = lv_arc_create(top);
        style_arc(v->level);
        lv_obj_set_style_arc_color(v->level, lv_color_hex(LEVEL_COLOR), LV_PART_INDICATOR);
        lv_arc_set_bg_angles(v->level, 0, 360);

        v->spinner = lv_spinner_create(top);
        style_arc(v->spinner);
        lv_obj_set_style_arc_color(v->spinner, lv_color_hex(THINKING_COLOR), LV_PART_INDICATOR);
        lv_spinner_set_anim_params(v->spinner, SPINNER_PERIOD_MS, SPINNER_ARC_DEG);
    }
    // Over the ring: when the eyes are open they cover the whole screen.
    ui_eyes_create(displays);
}

static void show_ring(voice_state_t state)
{
    ui_eyes_set_busy(state != VOICE_IDLE);
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        ui_voice_t *v = &s_voice[i];
        bool ring = state == VOICE_LISTENING || state == VOICE_SPEAKING || state == VOICE_ERROR;
        if (ring) {
            uint32_t color = state == VOICE_LISTENING  ? LISTENING_COLOR
                             : state == VOICE_SPEAKING ? SPEAKING_COLOR
                                                       : ERROR_COLOR;
            lv_obj_set_style_arc_color(v->ring, lv_color_hex(color), LV_PART_INDICATOR);
            lv_obj_remove_flag(v->ring, LV_OBJ_FLAG_HIDDEN);
        } else {
            lv_obj_add_flag(v->ring, LV_OBJ_FLAG_HIDDEN);
        }
        if (state != VOICE_LISTENING && state != VOICE_SPEAKING) {
            lv_obj_add_flag(v->level, LV_OBJ_FLAG_HIDDEN);
            s_level_step = 0;
        }
        if (state == VOICE_THINKING) {
            lv_obj_remove_flag(v->spinner, LV_OBJ_FLAG_HIDDEN);
        } else {
            lv_obj_add_flag(v->spinner, LV_OBJ_FLAG_HIDDEN);
        }
    }
}

void ui_voice_show(voice_state_t state)
{
    s_state = state;
    if (s_eyes) {
        ui_eyes_show(state);
    } else {
        show_ring(state);
    }
}

void ui_voice_set_eyes(bool on)
{
    if (on == s_eyes) {
        return;
    }
    s_eyes = on;
    if (on) {
        show_ring(VOICE_IDLE);
        ui_eyes_show(s_state);
    } else {
        ui_eyes_show(VOICE_IDLE);
        show_ring(s_state);
    }
}

void ui_voice_set_level(float level)
{
    if (s_eyes) {
        ui_eyes_set_level(level);
        return;
    }
    int step = (int) (level * LEVEL_STEPS + 0.5f);
    if (step == s_level_step) {
        return;
    }
    s_level_step = step;
    int half = step * LEVEL_MAX_DEG / LEVEL_STEPS / 2;
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        ui_voice_t *v = &s_voice[i];
        if (step == 0) {
            lv_obj_add_flag(v->level, LV_OBJ_FLAG_HIDDEN);
            continue;
        }
        lv_arc_set_angles(v->level, (LEVEL_CENTER_DEG - half + 360) % 360, (LEVEL_CENTER_DEG + half) % 360);
        lv_obj_remove_flag(v->level, LV_OBJ_FLAG_HIDDEN);
    }
}
