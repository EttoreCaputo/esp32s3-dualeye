#include "ui_watch.h"

#include <stdio.h>
#include <string.h>

#include "board_display.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "media.h"

LV_FONT_DECLARE(lv_font_montserrat_bold_12)
LV_FONT_DECLARE(lv_font_montserrat_bold_48)
LV_FONT_DECLARE(lv_font_fan_16)

/* Font Awesome 6 Free solid "fan", U+F863 */
#define UI_SYMBOL_FAN "\xEF\xA1\xA3"

/* Missing values: the large fonts carry an em dash, the built-in 14 px one is
 * ASCII only, so small labels use "--". */
#define COLOR_BG 0x000000
#define COLOR_TEXT 0xFFFFFF
#define COLOR_TEXT_DIM 0x9A9A9C
#define COLOR_CYAN 0x3AE7ED
#define COLOR_TEMP_TRACK 0x0B2C30
#define COLOR_MEM 0x5E8BFF
#define COLOR_MEM_TRACK 0x141D3A
#define COLOR_USAGE_CPU 0xC4F06A
#define COLOR_USAGE_GPU 0xC86CF0
#define COLOR_TRACK_CPU 0x163012
#define COLOR_TRACK_GPU 0x2A1238
#define COLOR_WARM 0xF8A639
#define COLOR_HOT 0xF05354
#define COLOR_ERROR 0xFF453A
#define COLOR_STALE 0xFFD60A
/* Claude's clay, a dimmer one for Clawd asleep, and the weekly ring's sand. */
#define COLOR_CLAUDE 0xD97757
#define COLOR_CLAUDE_DIM 0x6E3B2B
#define COLOR_CLAUDE_TRACK 0x35190F
#define COLOR_WEEK 0xE9C4A6
#define COLOR_WEEK_TRACK 0x2B2019
/* Upload on the net face, and the battery. */
#define COLOR_GREEN 0x40E080
#define COLOR_GREEN_TRACK 0x0F2A18

/* The net face's rings scale to the fastest of the last minute or so, never below 64 KB/s. */
#define NET_SCALE_MIN 65536.0f
#define NET_SCALE_DECAY 0.97f
#define DISK_FULL_PCT 90
#define BATTERY_LOW_PCT 20
#define BATTERY_EMPTY_PCT 10
#define IMAGE_TICK_MS 20

#define TEMP_WARM_C 80.0f
#define TEMP_HOT_C 90.0f
#define MEM_HIGH_PCT 90
/* Claude limits: orange from 80 %, red from 95 %. */
#define CLAUDE_WARM_PCT 80
#define CLAUDE_HOT_PCT 95
#define CLAUDE_BLOCK_MIN 300

#define USAGE_ARC_SIZE 216
#define RING_GAP 32
#define ARC_WIDTH 13

/* Clawd, Claude Code's mascot, on a 16 x 5 grid of CLAWD px-sized cells: a
 * body with two eye holes, arms one row across, four legs. */
#define CLAWD_COLS 16
#define CLAWD_ROWS 5
#define CLAWD_TICK_MS 150
#define CLAWD_BLINK_TICKS 24
#define CLAWD_SMALL_PX 4
#define CLAWD_LARGE_PX 8

static const char *TAG = "ui_watch";

typedef struct {
    lv_obj_t *warn;
    lv_obj_t *label;
} ui_title_t;

/* Temperature, clock, power, load ring and fan. The plus and bar faces add a
 * RAM or VRAM bar below (plus with its name and GiB); fields a face doesn't
 * have stay NULL. */
typedef struct {
    lv_obj_t *root;
    lv_obj_t *usage_arc;
    ui_title_t title;
    lv_obj_t *value;
    lv_obj_t *clock;
    lv_obj_t *watts;
    lv_obj_t *usage;
    lv_obj_t *rpm;
    lv_obj_t *mem_bar;
    lv_obj_t *mem_name;
    lv_obj_t *mem_value;
} ui_classic_t;

/* Activity-style rings, outside in: load, temperature, memory. */
typedef struct {
    lv_obj_t *root;
    lv_obj_t *usage_arc;
    lv_obj_t *temp_arc;
    lv_obj_t *mem_arc;
    ui_title_t title;
    lv_obj_t *value;
    lv_obj_t *usage;
    lv_obj_t *mem;
} ui_rings_t;

/* One Clawd; it animates while its face is showing, as its state says. */
typedef struct {
    lv_obj_t *face;
    lv_obj_t *root;
    lv_obj_t *body;
    lv_obj_t *arms;
    lv_obj_t *eyes[2];
    lv_obj_t *legs[4];
    lv_obj_t *zzz;
    int px;
    metrics_claude_state_t state;
    uint32_t color;
} ui_clawd_t;

/* Claude usage: 5-hour limit ring outside, weekly ring inside, a small Clawd
 * over the 5-hour share (or the window's tokens without the status line). */
typedef struct {
    lv_obj_t *root;
    lv_obj_t *session_arc;
    lv_obj_t *week_arc;
    ui_clawd_t clawd;
    lv_obj_t *value;
    lv_obj_t *reset;
    lv_obj_t *week_name;
    lv_obj_t *week;
} ui_claude_t;

/* Download on the outer ring and in large, upload inside. */
typedef struct {
    lv_obj_t *root;
    lv_obj_t *rx_arc;
    lv_obj_t *tx_arc;
    ui_title_t title;
    lv_obj_t *value;
    lv_obj_t *unit;
    lv_obj_t *tx;
    float rx_peak;
    float tx_peak;
} ui_net_t;

/* The system disk: space used on the ring, reads and writes below. */
typedef struct {
    lv_obj_t *root;
    lv_obj_t *arc;
    ui_title_t title;
    lv_obj_t *value;
    lv_obj_t *space;
    lv_obj_t *io_row;
    lv_obj_t *read;
    lv_obj_t *write;
} ui_disk_t;

/* Charge on the ring, what the battery is doing and for how long. */
typedef struct {
    lv_obj_t *root;
    lv_obj_t *arc;
    ui_title_t title;
    lv_obj_t *value;
    lv_obj_t *status;
    lv_obj_t *time;
} ui_battery_t;

/* The uploaded picture, a frame at a time, or a hint without one. */
typedef struct {
    lv_obj_t *root;
    lv_obj_t *img;
    lv_obj_t *hint;
    lv_image_dsc_t dsc;
    uint16_t *pixels;
    uint32_t generation;
    int frame;
    int frames;
    uint32_t next_ms;
} ui_image_face_t;

/* A large Clawd between the model name and what Claude is doing. */
typedef struct {
    lv_obj_t *root;
    lv_obj_t *session_arc;
    lv_obj_t *model;
    ui_clawd_t clawd;
    lv_obj_t *status;
    lv_obj_t *tokens;
} ui_clawd_face_t;

typedef struct {
    lv_obj_t *screen;
    /* UI_SCREEN_*: which image slot is ours. */
    int index;
    /* Whose metrics classic, rings, plus and bar show, and how they're named and coloured. */
    metrics_source_t source;
    const char *name;
    const char *mem_name;
    uint32_t accent;
    uint32_t track;
    ui_classic_t classic;
    ui_rings_t rings;
    ui_classic_t plus;
    ui_classic_t bar;
    ui_claude_t claude;
    ui_clawd_face_t clawd;
    ui_net_t net;
    ui_disk_t disk;
    ui_battery_t battery;
    ui_image_face_t image;
} ui_screen_t;

/* How the classic-based faces differ: column offset, gap under the title and
 * the memory bar (none when bar_w is 0). */
typedef struct {
    int y_ofs;
    int title_gap;
    int bar_w;
    int bar_h;
    bool mem_text;
} ui_classic_layout_t;

static const ui_classic_layout_t LAYOUT_CLASSIC = {.y_ofs = 2, .title_gap = 10};
static const ui_classic_layout_t LAYOUT_PLUS = {.y_ofs = -10, .title_gap = 8, .bar_w = 96, .bar_h = 6, .mem_text = true};
static const ui_classic_layout_t LAYOUT_BAR = {.y_ofs = -3, .title_gap = 10, .bar_w = 72, .bar_h = 4};

/* Label/value colours and the warning icon, from temperature and link state. */
typedef struct {
    uint32_t label;
    uint32_t value;
    bool warn;
} ui_tone_t;

static ui_screen_t s_cpu;
static ui_screen_t s_gpu;
static uint32_t s_clawd_tick;

static void style_screen_black(lv_obj_t *screen)
{
    lv_obj_set_style_bg_color(screen, lv_color_hex(COLOR_BG), 0);
    lv_obj_set_style_bg_opa(screen, LV_OPA_COVER, 0);
    lv_obj_clear_flag(screen, LV_OBJ_FLAG_SCROLLABLE);
}

static lv_obj_t *create_text(lv_obj_t *parent, const char *text, const lv_font_t *font, uint32_t color)
{
    lv_obj_t *label = lv_label_create(parent);
    lv_label_set_text(label, text);
    lv_obj_set_style_text_font(label, font, 0);
    lv_obj_set_style_text_color(label, lv_color_hex(color), 0);
    lv_obj_set_style_text_align(label, LV_TEXT_ALIGN_CENTER, 0);
    return label;
}

static void set_text_color(lv_obj_t *label, uint32_t color)
{
    lv_obj_set_style_text_color(label, lv_color_hex(color), 0);
}

static lv_obj_t *make_flex(lv_obj_t *parent, lv_flex_flow_t flow)
{
    lv_obj_t *obj = lv_obj_create(parent);
    lv_obj_remove_style_all(obj);
    lv_obj_set_size(obj, LV_SIZE_CONTENT, LV_SIZE_CONTENT);
    lv_obj_set_flex_flow(obj, flow);
    lv_obj_set_flex_align(obj, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    lv_obj_clear_flag(obj, LV_OBJ_FLAG_SCROLLABLE);
    return obj;
}

/* A transparent full-screen layer holding one face. */
static lv_obj_t *make_face(lv_obj_t *screen)
{
    lv_obj_t *obj = lv_obj_create(screen);
    lv_obj_remove_style_all(obj);
    lv_obj_set_size(obj, LV_PCT(100), LV_PCT(100));
    lv_obj_clear_flag(obj, LV_OBJ_FLAG_SCROLLABLE | LV_OBJ_FLAG_CLICKABLE);
    lv_obj_add_flag(obj, LV_OBJ_FLAG_HIDDEN);
    return obj;
}

static void style_arc(lv_obj_t *arc, int size, uint32_t color, uint32_t track)
{
    lv_obj_set_size(arc, size, size);
    lv_obj_align(arc, LV_ALIGN_CENTER, 0, 0);
    lv_arc_set_rotation(arc, 270);
    lv_arc_set_bg_angles(arc, 0, 360);
    lv_arc_set_range(arc, 0, 100);
    lv_arc_set_value(arc, 0);
    lv_arc_set_mode(arc, LV_ARC_MODE_NORMAL);
    lv_obj_remove_style(arc, NULL, LV_PART_KNOB);
    lv_obj_clear_flag(arc, LV_OBJ_FLAG_CLICKABLE);

    lv_obj_set_style_arc_width(arc, ARC_WIDTH, LV_PART_MAIN);
    lv_obj_set_style_arc_color(arc, lv_color_hex(track), LV_PART_MAIN);
    lv_obj_set_style_arc_rounded(arc, true, LV_PART_MAIN);
    lv_obj_set_style_arc_width(arc, ARC_WIDTH, LV_PART_INDICATOR);
    lv_obj_set_style_arc_color(arc, lv_color_hex(color), LV_PART_INDICATOR);
    lv_obj_set_style_arc_rounded(arc, true, LV_PART_INDICATOR);
}

static lv_obj_t *create_arc(lv_obj_t *parent, int size, uint32_t color, uint32_t track)
{
    lv_obj_t *arc = lv_arc_create(parent);
    style_arc(arc, size, color, track);
    return arc;
}

static void set_arc_color(lv_obj_t *arc, uint32_t color)
{
    lv_obj_set_style_arc_color(arc, lv_color_hex(color), LV_PART_INDICATOR);
}

static lv_obj_t *create_fan(lv_obj_t *parent)
{
    return create_text(parent, UI_SYMBOL_FAN, &lv_font_fan_16, COLOR_TEXT);
}

static lv_obj_t *create_column(lv_obj_t *parent, int y_ofs)
{
    lv_obj_t *col = make_flex(parent, LV_FLEX_FLOW_COLUMN);
    lv_obj_set_style_pad_row(col, 0, 0);
    lv_obj_align(col, LV_ALIGN_CENTER, 0, y_ofs);
    return col;
}

static void create_title(ui_title_t *title, lv_obj_t *col, const char *text, uint32_t color, int margin_bottom)
{
    lv_obj_t *row = make_flex(col, LV_FLEX_FLOW_ROW);
    lv_obj_set_style_pad_column(row, 4, 0);
    lv_obj_set_style_margin_bottom(row, margin_bottom, 0);
    title->warn = create_text(row, LV_SYMBOL_WARNING, &lv_font_montserrat_12, color);
    lv_obj_add_flag(title->warn, LV_OBJ_FLAG_HIDDEN);
    title->label = create_text(row, text, &lv_font_montserrat_bold_12, color);
    lv_obj_set_style_text_letter_space(title->label, 1, 0);
}

static void set_title(ui_title_t *title, uint32_t color, bool warn)
{
    set_text_color(title->label, color);
    if (warn) {
        set_text_color(title->warn, color);
        lv_obj_remove_flag(title->warn, LV_OBJ_FLAG_HIDDEN);
    } else {
        lv_obj_add_flag(title->warn, LV_OBJ_FLAG_HIDDEN);
    }
}

static lv_obj_t *create_bar(lv_obj_t *parent, int w, int h, uint32_t color, uint32_t track)
{
    lv_obj_t *bar = lv_bar_create(parent);
    lv_obj_set_size(bar, w, h);
    lv_bar_set_range(bar, 0, 100);
    lv_bar_set_value(bar, 0, LV_ANIM_OFF);
    lv_obj_set_style_radius(bar, LV_RADIUS_CIRCLE, LV_PART_MAIN);
    lv_obj_set_style_radius(bar, LV_RADIUS_CIRCLE, LV_PART_INDICATOR);
    lv_obj_set_style_bg_color(bar, lv_color_hex(track), LV_PART_MAIN);
    lv_obj_set_style_bg_opa(bar, LV_OPA_COVER, LV_PART_MAIN);
    lv_obj_set_style_bg_color(bar, lv_color_hex(color), LV_PART_INDICATOR);
    lv_obj_set_style_bg_opa(bar, LV_OPA_COVER, LV_PART_INDICATOR);
    lv_obj_clear_flag(bar, LV_OBJ_FLAG_CLICKABLE);
    return bar;
}

static void create_classic(ui_screen_t *ui, ui_classic_t *f, const ui_classic_layout_t *layout)
{
    f->root = make_face(ui->screen);
    f->usage_arc = create_arc(f->root, USAGE_ARC_SIZE, ui->accent, ui->track);

    lv_obj_t *col = create_column(f->root, layout->y_ofs);
    create_title(&f->title, col, ui->name, ui->accent, layout->title_gap);

    f->value = create_text(col, "—", &lv_font_montserrat_bold_48, COLOR_TEXT);
    lv_obj_set_style_margin_bottom(f->value, 2, 0);

    lv_obj_t *clock_row = make_flex(col, LV_FLEX_FLOW_ROW);
    lv_obj_set_style_margin_top(clock_row, 4, 0);
    lv_obj_set_style_pad_column(clock_row, 8, 0);
    f->clock = create_text(clock_row, "-- GHz", &lv_font_montserrat_14, COLOR_TEXT_DIM);
    f->watts = create_text(clock_row, "-- W", &lv_font_montserrat_14, COLOR_TEXT_DIM);

    lv_obj_t *load_row = make_flex(col, LV_FLEX_FLOW_ROW);
    lv_obj_set_style_pad_column(load_row, 6, 0);
    lv_obj_set_style_margin_top(load_row, 2, 0);
    lv_obj_set_style_margin_bottom(load_row, 3, 0);
    f->usage = create_text(load_row, "--%", &lv_font_montserrat_14, ui->accent);
    create_fan(load_row);
    f->rpm = create_text(load_row, "--", &lv_font_montserrat_14, COLOR_TEXT);

    if (layout->bar_w == 0) {
        return;
    }
    f->mem_bar = create_bar(col, layout->bar_w, layout->bar_h, COLOR_MEM, COLOR_MEM_TRACK);
    lv_obj_set_style_margin_top(f->mem_bar, 6, 0);
    if (!layout->mem_text) {
        return;
    }

    lv_obj_t *mem_row = make_flex(col, LV_FLEX_FLOW_ROW);
    lv_obj_set_style_pad_column(mem_row, 6, 0);
    lv_obj_set_style_margin_top(mem_row, 5, 0);
    f->mem_name = create_text(mem_row, ui->mem_name, &lv_font_montserrat_bold_12, COLOR_MEM);
    lv_obj_set_style_text_letter_space(f->mem_name, 1, 0);
    f->mem_value = create_text(mem_row, "-- GB", &lv_font_montserrat_14, COLOR_TEXT_DIM);
}

static void create_rings(ui_screen_t *ui)
{
    ui_rings_t *f = &ui->rings;
    f->root = make_face(ui->screen);
    f->usage_arc = create_arc(f->root, USAGE_ARC_SIZE, ui->accent, ui->track);
    f->temp_arc = create_arc(f->root, USAGE_ARC_SIZE - RING_GAP, COLOR_CYAN, COLOR_TEMP_TRACK);
    f->mem_arc = create_arc(f->root, USAGE_ARC_SIZE - 2 * RING_GAP, COLOR_MEM, COLOR_MEM_TRACK);

    lv_obj_t *col = create_column(f->root, 2);
    create_title(&f->title, col, ui->name, ui->accent, 6);
    f->value = create_text(col, "—", &lv_font_montserrat_bold_48, COLOR_TEXT);

    lv_obj_t *row = make_flex(col, LV_FLEX_FLOW_ROW);
    lv_obj_set_style_pad_column(row, 8, 0);
    lv_obj_set_style_margin_top(row, 6, 0);
    f->usage = create_text(row, "--%", &lv_font_montserrat_14, ui->accent);
    f->mem = create_text(row, "--%", &lv_font_montserrat_14, COLOR_MEM);
}

static lv_obj_t *create_cell(lv_obj_t *parent, uint32_t color)
{
    lv_obj_t *cell = lv_obj_create(parent);
    lv_obj_remove_style_all(cell);
    lv_obj_set_style_bg_color(cell, lv_color_hex(color), 0);
    lv_obj_set_style_bg_opa(cell, LV_OPA_COVER, 0);
    lv_obj_clear_flag(cell, LV_OBJ_FLAG_SCROLLABLE | LV_OBJ_FLAG_CLICKABLE);
    return cell;
}

static void place(lv_obj_t *obj, int x, int y, int w, int h)
{
    lv_obj_set_pos(obj, x, y);
    lv_obj_set_size(obj, w, h);
}

/* Lay Clawd out: `bob` lifts the body, a lifted leg is half as long, and
 * `eye_h` below px narrows the eyes to a slit (at the bottom when asleep). */
static void clawd_pose(ui_clawd_t *c, int bob, bool lift_a, bool lift_b, int eye_h, bool eyes_low)
{
    static const int EYE_COL[2] = {4, 11};
    static const int LEG_COL[4] = {3, 5, 10, 12};
    int px = c->px;
    place(c->body, 2 * px, -bob, 12 * px, 4 * px);
    place(c->arms, 0, 2 * px - bob, CLAWD_COLS * px, px);
    for (int i = 0; i < 2; i++) {
        int y = px - bob + (eyes_low ? px - eye_h : (px - eye_h) / 2);
        place(c->eyes[i], EYE_COL[i] * px, y, px, eye_h);
    }
    for (int i = 0; i < 4; i++) {
        bool lifted = (i % 2 == 0) ? lift_a : lift_b;
        place(c->legs[i], LEG_COL[i] * px, 4 * px, px, lifted ? px / 2 : px);
    }
}

static void create_clawd(ui_clawd_t *c, lv_obj_t *face, lv_obj_t *parent, int px)
{
    c->face = face;
    c->px = px;
    c->color = COLOR_CLAUDE;
    c->state = METRICS_CLAUDE_SLEEP;
    c->root = lv_obj_create(parent);
    lv_obj_remove_style_all(c->root);
    lv_obj_set_size(c->root, CLAWD_COLS * px, CLAWD_ROWS * px);
    lv_obj_clear_flag(c->root, LV_OBJ_FLAG_SCROLLABLE | LV_OBJ_FLAG_CLICKABLE);
    lv_obj_add_flag(c->root, LV_OBJ_FLAG_OVERFLOW_VISIBLE);

    c->body = create_cell(c->root, c->color);
    c->arms = create_cell(c->root, c->color);
    for (int i = 0; i < 2; i++) {
        c->eyes[i] = create_cell(c->root, COLOR_BG);
    }
    for (int i = 0; i < 4; i++) {
        c->legs[i] = create_cell(c->root, c->color);
    }
    c->zzz = NULL;
    if (px >= CLAWD_LARGE_PX) {
        c->zzz = create_text(c->root, "", &lv_font_montserrat_14, COLOR_TEXT_DIM);
        lv_obj_set_pos(c->zzz, CLAWD_COLS * px - px, -2 * px);
    }
    clawd_pose(c, 0, false, false, px, false);
}

static void clawd_set_color(ui_clawd_t *c, uint32_t color)
{
    if (c->color == color) {
        return;
    }
    c->color = color;
    lv_obj_t *cells[] = {c->body, c->arms, c->legs[0], c->legs[1], c->legs[2], c->legs[3]};
    for (size_t i = 0; i < sizeof(cells) / sizeof(cells[0]); i++) {
        lv_obj_set_style_bg_color(cells[i], lv_color_hex(color), 0);
    }
}

/* Working: walks in place with a bob. Idle: blinks now and then. Asleep:
 * eyes shut, dimmer, snoring on the large one. */
static void clawd_animate(ui_clawd_t *c, uint32_t tick)
{
    if (c->root == NULL || lv_obj_has_flag(c->face, LV_OBJ_FLAG_HIDDEN)) {
        return;
    }
    int px = c->px;
    int slit = px / 4 > 0 ? px / 4 : 1;
    int bob = px / 4 > 0 ? px / 4 : 1;
    const char *zzz = "";
    switch (c->state) {
    case METRICS_CLAUDE_WORK: {
        uint32_t phase = tick % 4;
        clawd_pose(c, (phase % 2) ? bob : 0, phase < 2, phase >= 2, px, false);
        break;
    }
    case METRICS_CLAUDE_IDLE:
        clawd_pose(c, 0, false, false, (tick % CLAWD_BLINK_TICKS == 0) ? slit : px, false);
        break;
    default: {
        static const char *const SNORE[] = {"z", "z Z", "z Z z", ""};
        clawd_pose(c, 0, false, false, slit, true);
        zzz = SNORE[(tick / 5) % 4];
        break;
    }
    }
    if (c->zzz != NULL && strcmp(lv_label_get_text(c->zzz), zzz) != 0) {
        lv_label_set_text(c->zzz, zzz);
    }
}

static void clawd_set_state(ui_clawd_t *c, metrics_claude_state_t state, bool placeholder)
{
    c->state = placeholder ? METRICS_CLAUDE_IDLE : state;
    bool dim = placeholder || state == METRICS_CLAUDE_SLEEP;
    clawd_set_color(c, dim ? COLOR_CLAUDE_DIM : COLOR_CLAUDE);
    clawd_animate(c, s_clawd_tick);
}

static void clawd_timer_cb(lv_timer_t *timer)
{
    (void) timer;
    s_clawd_tick++;
    ui_screen_t *screens[] = {&s_cpu, &s_gpu};
    for (int i = 0; i < 2; i++) {
        clawd_animate(&screens[i]->claude.clawd, s_clawd_tick);
        clawd_animate(&screens[i]->clawd.clawd, s_clawd_tick);
    }
}

/* A bold 12 name and a 14 px value side by side. */
static lv_obj_t *create_pair(lv_obj_t *col, lv_obj_t **name, const char *name_text, uint32_t name_color,
                             lv_obj_t **value)
{
    lv_obj_t *row = make_flex(col, LV_FLEX_FLOW_ROW);
    lv_obj_set_style_pad_column(row, 6, 0);
    lv_obj_t *label = create_text(row, name_text, &lv_font_montserrat_bold_12, name_color);
    lv_obj_set_style_text_letter_space(label, 1, 0);
    if (name != NULL) {
        *name = label;
    }
    *value = create_text(row, "--", &lv_font_montserrat_14, COLOR_TEXT_DIM);
    return row;
}

static void create_claude(ui_screen_t *ui)
{
    ui_claude_t *f = &ui->claude;
    f->root = make_face(ui->screen);
    f->session_arc = create_arc(f->root, USAGE_ARC_SIZE, COLOR_CLAUDE, COLOR_CLAUDE_TRACK);
    f->week_arc = create_arc(f->root, USAGE_ARC_SIZE - RING_GAP, COLOR_WEEK, COLOR_WEEK_TRACK);

    lv_obj_t *col = create_column(f->root, 0);
    create_clawd(&f->clawd, f->root, col, CLAWD_SMALL_PX);
    lv_obj_set_style_margin_bottom(f->clawd.root, 8, 0);
    f->value = create_text(col, "—", &lv_font_montserrat_bold_48, COLOR_TEXT_DIM);
    lv_obj_set_style_margin_bottom(f->value, 6, 0);
    create_pair(col, NULL, "5H", COLOR_CLAUDE, &f->reset);
    lv_obj_t *week_row = create_pair(col, &f->week_name, "WK", COLOR_WEEK, &f->week);
    lv_obj_set_style_margin_top(week_row, 2, 0);
}

static void create_clawd_face(ui_screen_t *ui)
{
    ui_clawd_face_t *f = &ui->clawd;
    f->root = make_face(ui->screen);
    f->session_arc = create_arc(f->root, USAGE_ARC_SIZE, COLOR_CLAUDE, COLOR_CLAUDE_TRACK);

    lv_obj_t *col = create_column(f->root, 2);
    f->model = create_text(col, "CLAUDE", &lv_font_montserrat_bold_12, COLOR_TEXT_DIM);
    lv_obj_set_style_text_letter_space(f->model, 1, 0);
    lv_obj_set_style_margin_bottom(f->model, 14, 0);
    create_clawd(&f->clawd, f->root, col, CLAWD_LARGE_PX);
    lv_obj_set_style_margin_bottom(f->clawd.root, 14, 0);
    lv_obj_t *row = create_pair(col, &f->status, "ASLEEP", COLOR_TEXT_DIM, &f->tokens);
    (void) row;
}

static void create_net(ui_screen_t *ui)
{
    ui_net_t *f = &ui->net;
    f->root = make_face(ui->screen);
    f->rx_arc = create_arc(f->root, USAGE_ARC_SIZE, COLOR_CYAN, COLOR_TEMP_TRACK);
    f->tx_arc = create_arc(f->root, USAGE_ARC_SIZE - RING_GAP, COLOR_GREEN, COLOR_GREEN_TRACK);
    f->rx_peak = NET_SCALE_MIN;
    f->tx_peak = NET_SCALE_MIN;

    lv_obj_t *col = create_column(f->root, 2);
    create_title(&f->title, col, "NET", COLOR_CYAN, 6);
    f->value = create_text(col, "—", &lv_font_montserrat_bold_48, COLOR_TEXT);
    f->unit = create_text(col, LV_SYMBOL_DOWN " --", &lv_font_montserrat_14, COLOR_CYAN);
    lv_obj_set_style_margin_bottom(f->unit, 4, 0);
    f->tx = create_text(col, LV_SYMBOL_UP " --", &lv_font_montserrat_14, COLOR_GREEN);
}

static void create_disk(ui_screen_t *ui)
{
    ui_disk_t *f = &ui->disk;
    f->root = make_face(ui->screen);
    f->arc = create_arc(f->root, USAGE_ARC_SIZE, COLOR_MEM, COLOR_MEM_TRACK);

    lv_obj_t *col = create_column(f->root, 2);
    create_title(&f->title, col, "DISK", COLOR_MEM, 6);
    f->value = create_text(col, "—", &lv_font_montserrat_bold_48, COLOR_TEXT);
    f->space = create_text(col, "-- GB", &lv_font_montserrat_14, COLOR_TEXT_DIM);
    lv_obj_set_style_margin_bottom(f->space, 4, 0);
    f->io_row = make_flex(col, LV_FLEX_FLOW_ROW);
    lv_obj_set_style_pad_column(f->io_row, 10, 0);
    lv_obj_t *read_name = NULL;
    lv_obj_t *write_name = NULL;
    lv_obj_t *r = create_pair(f->io_row, &read_name, "R", COLOR_MEM, &f->read);
    lv_obj_t *w = create_pair(f->io_row, &write_name, "W", COLOR_WARM, &f->write);
    (void) r;
    (void) w;
}

static void create_battery(ui_screen_t *ui)
{
    ui_battery_t *f = &ui->battery;
    f->root = make_face(ui->screen);
    f->arc = create_arc(f->root, USAGE_ARC_SIZE, COLOR_GREEN, COLOR_GREEN_TRACK);

    lv_obj_t *col = create_column(f->root, 2);
    create_title(&f->title, col, "BATTERY", COLOR_GREEN, 6);
    f->value = create_text(col, "—", &lv_font_montserrat_bold_48, COLOR_TEXT);
    f->status = create_text(col, "NO BATTERY", &lv_font_montserrat_bold_12, COLOR_TEXT_DIM);
    lv_obj_set_style_text_letter_space(f->status, 1, 0);
    lv_obj_set_style_margin_top(f->status, 6, 0);
    lv_obj_set_style_margin_bottom(f->status, 4, 0);
    f->time = create_text(col, "", &lv_font_montserrat_14, COLOR_TEXT_DIM);
}

static void create_image(ui_screen_t *ui)
{
    ui_image_face_t *f = &ui->image;
    f->root = make_face(ui->screen);
    f->img = lv_image_create(f->root);
    lv_obj_center(f->img);
    lv_obj_add_flag(f->img, LV_OBJ_FLAG_HIDDEN);
    lv_obj_t *col = create_column(f->root, 0);
    f->hint = col;
    lv_obj_t *title = create_text(col, "NO IMAGE", &lv_font_montserrat_bold_12, COLOR_TEXT_DIM);
    lv_obj_set_style_text_letter_space(title, 1, 0);
    lv_obj_set_style_margin_bottom(title, 6, 0);
    create_text(col, "Pick one in the\nDualEye app", &lv_font_montserrat_14, COLOR_TEXT_DIM);
    // Not loaded yet: the first update looks.
    f->generation = UINT32_MAX;
}

/* Name and colour the classic, rings, plus and bar faces after `source`. */
static void apply_source(ui_screen_t *ui, metrics_source_t source)
{
    if (source == ui->source && ui->name != NULL) {
        return;
    }
    bool gpu = source == METRICS_SOURCE_GPU;
    ui->source = source;
    ui->name = gpu ? "GPU" : "CPU";
    ui->mem_name = gpu ? "VRAM" : "RAM";
    ui->accent = gpu ? COLOR_USAGE_GPU : COLOR_USAGE_CPU;
    ui->track = gpu ? COLOR_TRACK_GPU : COLOR_TRACK_CPU;
    ui_classic_t *classics[] = {&ui->classic, &ui->plus, &ui->bar};
    for (size_t i = 0; i < sizeof(classics) / sizeof(classics[0]); i++) {
        if (classics[i]->root == NULL) {
            continue;
        }
        style_arc(classics[i]->usage_arc, USAGE_ARC_SIZE, ui->accent, ui->track);
        lv_label_set_text(classics[i]->title.label, ui->name);
        if (classics[i]->mem_name != NULL) {
            lv_label_set_text(classics[i]->mem_name, ui->mem_name);
        }
    }
    if (ui->rings.root != NULL) {
        style_arc(ui->rings.usage_arc, USAGE_ARC_SIZE, ui->accent, ui->track);
        lv_label_set_text(ui->rings.title.label, ui->name);
    }
}

static void create_screen(ui_screen_t *ui, lv_display_t *disp, int index, metrics_source_t source)
{
    ui->index = index;
    apply_source(ui, source);
    ui->screen = lv_display_get_screen_active(disp);
    style_screen_black(ui->screen);

    create_classic(ui, &ui->classic, &LAYOUT_CLASSIC);
    create_rings(ui);
    create_classic(ui, &ui->plus, &LAYOUT_PLUS);
    create_classic(ui, &ui->bar, &LAYOUT_BAR);
    create_claude(ui);
    create_clawd_face(ui);
    create_net(ui);
    create_disk(ui);
    create_battery(ui);
    create_image(ui);
    lv_obj_remove_flag(ui->classic.root, LV_OBJ_FLAG_HIDDEN);
}

static int clamp_pct(float value, float max_value)
{
    if (max_value <= 0.0f) {
        max_value = 100.0f;
    }
    int pct = (int) ((value / max_value) * 100.0f + 0.5f);
    if (pct < 0) {
        return 0;
    }
    return pct > 100 ? 100 : pct;
}

static int mem_pct(const metrics_temp_t *m)
{
    return m->mem_valid ? clamp_pct(m->mem_used_mb, m->mem_total_mb) : 0;
}

/* From 80 C the label and temperature go orange, from 90 C red. Rings stay put. */
static ui_tone_t temp_tone(const ui_screen_t *ui, float temp_c, metrics_ui_state_t state)
{
    ui_tone_t tone = {.label = ui->accent, .value = COLOR_TEXT, .warn = false};
    if (temp_c >= TEMP_HOT_C) {
        tone = (ui_tone_t) {.label = COLOR_HOT, .value = COLOR_HOT, .warn = true};
    } else if (temp_c >= TEMP_WARM_C) {
        tone = (ui_tone_t) {.label = COLOR_WARM, .value = COLOR_WARM, .warn = true};
    } else if (state == METRICS_UI_STALE) {
        tone = (ui_tone_t) {.label = COLOR_STALE, .value = COLOR_TEXT_DIM, .warn = false};
    } else if (state == METRICS_UI_ERROR) {
        tone.label = COLOR_ERROR;
    }
    return tone;
}

/* The temperature rings follow the same thresholds, cyan below them. */
static uint32_t temp_ring_color(float temp_c)
{
    if (temp_c >= TEMP_HOT_C) {
        return COLOR_HOT;
    }
    return temp_c >= TEMP_WARM_C ? COLOR_WARM : COLOR_CYAN;
}

static void set_temp_value(lv_obj_t *label, float temp_c)
{
    char value[16];
    snprintf(value, sizeof(value), "%d°", (int) (temp_c + 0.5f));
    lv_label_set_text(label, value);
}

static void set_pct(lv_obj_t *label, bool valid, int pct)
{
    if (!valid) {
        lv_label_set_text(label, "--%");
        return;
    }
    char text[8];
    snprintf(text, sizeof(text), "%d%%", pct);
    lv_label_set_text(label, text);
}

static uint32_t placeholder_title(metrics_ui_state_t state)
{
    return state == METRICS_UI_ERROR ? COLOR_ERROR : COLOR_TEXT_DIM;
}

/* Share of RAM or VRAM on the bar, used and total GiB beside the name; orange
 * when nearly full, like a warm temperature. */
static void update_mem_bar(ui_classic_t *f, const metrics_temp_t *temp)
{
    int pct = mem_pct(temp);
    uint32_t color = pct >= MEM_HIGH_PCT ? COLOR_WARM : COLOR_MEM;
    lv_bar_set_value(f->mem_bar, pct, LV_ANIM_OFF);
    lv_obj_set_style_bg_color(f->mem_bar, lv_color_hex(color), LV_PART_INDICATOR);
    if (f->mem_value == NULL) {
        return;
    }
    if (!temp->mem_valid) {
        lv_label_set_text(f->mem_value, "-- GB");
        set_text_color(f->mem_name, COLOR_TEXT_DIM);
        return;
    }
    set_text_color(f->mem_name, color);

    char text[24];
    snprintf(text, sizeof(text), "%.1f/%.0f GB", temp->mem_used_mb / 1024.0f, temp->mem_total_mb / 1024.0f);
    lv_label_set_text(f->mem_value, text);
}

static void update_classic(ui_screen_t *ui, ui_classic_t *f, const metrics_temp_t *temp, metrics_ui_state_t state,
                           int fan_rpm)
{
    if (state == METRICS_UI_WAITING || !temp->valid) {
        lv_arc_set_value(f->usage_arc, 0);
        lv_label_set_text(f->value, "—");
        lv_label_set_text(f->clock, "-- GHz");
        lv_label_set_text(f->watts, "-- W");
        lv_label_set_text(f->usage, "--%");
        lv_label_set_text(f->rpm, "--");
        set_title(&f->title, placeholder_title(state), false);
        set_text_color(f->value, COLOR_TEXT_DIM);
        set_text_color(f->usage, COLOR_TEXT_DIM);
        if (f->mem_bar) {
            metrics_temp_t none = {0};
            update_mem_bar(f, &none);
        }
        return;
    }

    set_temp_value(f->value, temp->temp_c);

    char clock[16];
    snprintf(clock, sizeof(clock), "%.1f GHz", temp->clock_ghz);
    lv_label_set_text(f->clock, clock);

    char watts[16];
    snprintf(watts, sizeof(watts), "%.0f W", temp->power_w);
    lv_label_set_text(f->watts, watts);

    char usage[8];
    snprintf(usage, sizeof(usage), "%.0f%%", temp->usage_pct);
    lv_label_set_text(f->usage, usage);

    if (fan_rpm < 0) {
        lv_label_set_text(f->rpm, "--");
    } else {
        char rpm[12];
        snprintf(rpm, sizeof(rpm), "%d", fan_rpm);
        lv_label_set_text(f->rpm, rpm);
    }

    lv_arc_set_value(f->usage_arc, clamp_pct(temp->usage_pct, 100.0f));

    ui_tone_t tone = temp_tone(ui, temp->temp_c, state);
    set_title(&f->title, tone.label, tone.warn);
    set_text_color(f->value, tone.value);
    set_text_color(f->usage, ui->accent);
    if (f->mem_bar) {
        update_mem_bar(f, temp);
    }
}

static void update_rings(ui_screen_t *ui, const metrics_temp_t *temp, float temp_max, metrics_ui_state_t state)
{
    ui_rings_t *f = &ui->rings;
    if (state == METRICS_UI_WAITING || !temp->valid) {
        lv_arc_set_value(f->usage_arc, 0);
        lv_arc_set_value(f->temp_arc, 0);
        lv_arc_set_value(f->mem_arc, 0);
        lv_label_set_text(f->value, "—");
        lv_label_set_text(f->usage, "--%");
        lv_label_set_text(f->mem, "--%");
        set_title(&f->title, placeholder_title(state), false);
        set_text_color(f->value, COLOR_TEXT_DIM);
        set_text_color(f->usage, COLOR_TEXT_DIM);
        set_text_color(f->mem, COLOR_TEXT_DIM);
        return;
    }

    set_temp_value(f->value, temp->temp_c);
    int usage = clamp_pct(temp->usage_pct, 100.0f);
    set_pct(f->usage, true, usage);
    set_pct(f->mem, temp->mem_valid, mem_pct(temp));

    lv_arc_set_value(f->usage_arc, usage);
    lv_arc_set_value(f->temp_arc, clamp_pct(temp->temp_c, temp_max));
    lv_arc_set_value(f->mem_arc, mem_pct(temp));
    set_arc_color(f->temp_arc, temp_ring_color(temp->temp_c));

    ui_tone_t tone = temp_tone(ui, temp->temp_c, state);
    set_title(&f->title, tone.label, tone.warn);
    set_text_color(f->value, tone.value);
    set_text_color(f->usage, ui->accent);
    set_text_color(f->mem, temp->mem_valid ? COLOR_MEM : COLOR_TEXT_DIM);
}

/* "1.2M", "845K", "9.4K", "512": fits the 48 px font's K and M. */
static void format_tokens(char *out, size_t len, float tokens)
{
    if (tokens < 1000.0f) {
        snprintf(out, len, "%d", (int) tokens);
    } else if (tokens < 9950.0f) {
        snprintf(out, len, "%.1fK", tokens / 1e3f);
    } else if (tokens < 999500.0f) {
        snprintf(out, len, "%.0fK", tokens / 1e3f);
    } else if (tokens < 99950000.0f) {
        snprintf(out, len, "%.1fM", tokens / 1e6f);
    } else {
        snprintf(out, len, "%.0fM", tokens / 1e6f);
    }
}

static void set_tokens(lv_obj_t *label, float tokens)
{
    char text[16];
    format_tokens(text, sizeof(text), tokens);
    lv_label_set_text(label, text);
}

static uint32_t limit_color(int pct, uint32_t normal)
{
    if (pct >= CLAUDE_HOT_PCT) {
        return COLOR_HOT;
    }
    return pct >= CLAUDE_WARM_PCT ? COLOR_WARM : normal;
}

static bool claude_live(const metrics_claude_t *c, metrics_ui_state_t state)
{
    return state != METRICS_UI_WAITING && c->valid;
}

/* The outer ring: the 5-hour limit used, or else how far into the window we are. */
static void update_session_arc(lv_obj_t *arc, const metrics_claude_t *c)
{
    int pct = 0;
    if (c->has_session) {
        pct = clamp_pct(c->session_pct, 100.0f);
    } else if (c->has_left) {
        pct = clamp_pct((float) (CLAUDE_BLOCK_MIN - c->left_min), (float) CLAUDE_BLOCK_MIN);
    }
    lv_arc_set_value(arc, pct);
    set_arc_color(arc, c->has_session ? limit_color(pct, COLOR_CLAUDE) : COLOR_CLAUDE);
}

static void update_claude(ui_screen_t *ui, const metrics_claude_t *c, metrics_ui_state_t state)
{
    ui_claude_t *f = &ui->claude;
    bool live = claude_live(c, state);
    clawd_set_state(&f->clawd, c->state, !live);
    if (!live) {
        lv_arc_set_value(f->session_arc, 0);
        lv_arc_set_value(f->week_arc, 0);
        lv_label_set_text(f->value, "—");
        lv_label_set_text(f->reset, "--");
        lv_label_set_text(f->week, "--");
        set_text_color(f->value, COLOR_TEXT_DIM);
        return;
    }

    update_session_arc(f->session_arc, c);
    uint32_t value_color = COLOR_TEXT;
    if (c->has_session) {
        int pct = clamp_pct(c->session_pct, 100.0f);
        set_pct(f->value, true, pct);
        value_color = limit_color(pct, COLOR_TEXT);
    } else {
        set_tokens(f->value, c->tokens);
    }
    set_text_color(f->value, state == METRICS_UI_STALE ? COLOR_TEXT_DIM : value_color);

    if (!c->has_left) {
        lv_label_set_text(f->reset, "--");
    } else if (c->left_min >= 60) {
        lv_label_set_text_fmt(f->reset, "%dh %02dm", c->left_min / 60, c->left_min % 60);
    } else {
        lv_label_set_text_fmt(f->reset, "%dm", c->left_min);
    }

    /* Without the status line the inner ring has nothing to show: today's
     * tokens take the weekly row instead. */
    if (c->has_week) {
        int pct = clamp_pct(c->week_pct, 100.0f);
        lv_obj_remove_flag(f->week_arc, LV_OBJ_FLAG_HIDDEN);
        lv_arc_set_value(f->week_arc, pct);
        set_arc_color(f->week_arc, limit_color(pct, COLOR_WEEK));
        lv_label_set_text(f->week_name, "WK");
        set_pct(f->week, true, pct);
    } else {
        lv_obj_add_flag(f->week_arc, LV_OBJ_FLAG_HIDDEN);
        lv_label_set_text(f->week_name, "DAY");
        set_tokens(f->week, c->today);
    }
}

static void update_clawd_face(ui_screen_t *ui, const metrics_claude_t *c, metrics_ui_state_t state)
{
    ui_clawd_face_t *f = &ui->clawd;
    bool live = claude_live(c, state);
    clawd_set_state(&f->clawd, c->state, !live);
    if (!live) {
        lv_arc_set_value(f->session_arc, 0);
        lv_label_set_text(f->model, "CLAUDE");
        lv_label_set_text(f->status, state == METRICS_UI_WAITING ? "WAITING" : "NO DATA");
        set_text_color(f->status, placeholder_title(state));
        lv_label_set_text(f->tokens, "--");
        return;
    }

    update_session_arc(f->session_arc, c);
    lv_label_set_text(f->model, c->model[0] != '\0' ? c->model : "CLAUDE");
    static const char *const STATUS[] = {
        [METRICS_CLAUDE_SLEEP] = "ASLEEP",
        [METRICS_CLAUDE_WORK] = "WORKING",
        [METRICS_CLAUDE_IDLE] = "IDLE",
    };
    lv_label_set_text(f->status, STATUS[c->state]);
    set_text_color(f->status, c->state == METRICS_CLAUDE_WORK ? COLOR_CLAUDE : COLOR_TEXT_DIM);
    set_tokens(f->tokens, c->tokens);
}

/* "512 B/s", "9.4 KB/s", "12 MB/s": the number and its unit apart, for the large value. */
static void format_rate(float bps, char *num, size_t num_len, const char **unit)
{
    static const char *const UNITS[] = {"B/s", "KB/s", "MB/s", "GB/s"};
    int u = 0;
    while (bps >= 1000.0f && u < 3) {
        bps /= 1000.0f;
        u++;
    }
    if (bps < 9.95f && u > 0) {
        snprintf(num, num_len, "%.1f", bps);
    } else {
        snprintf(num, num_len, "%d", (int) (bps + 0.5f));
    }
    *unit = UNITS[u];
}

static void set_rate(lv_obj_t *label, const char *prefix, float bps)
{
    char num[12];
    const char *unit;
    format_rate(bps, num, sizeof(num), &unit);
    lv_label_set_text_fmt(label, "%s%s %s", prefix, num, unit);
}

/* A peak that follows `value` up at once and down slowly. */
static float follow_peak(float peak, float value)
{
    peak *= NET_SCALE_DECAY;
    if (value > peak) {
        peak = value;
    }
    return peak < NET_SCALE_MIN ? NET_SCALE_MIN : peak;
}

static void update_net(ui_screen_t *ui, const metrics_net_t *net, metrics_ui_state_t state)
{
    ui_net_t *f = &ui->net;
    if (state == METRICS_UI_WAITING || !net->valid) {
        lv_arc_set_value(f->rx_arc, 0);
        lv_arc_set_value(f->tx_arc, 0);
        lv_label_set_text(f->value, "—");
        lv_label_set_text(f->unit, LV_SYMBOL_DOWN " --");
        lv_label_set_text(f->tx, LV_SYMBOL_UP " --");
        set_title(&f->title, placeholder_title(state), false);
        set_text_color(f->value, COLOR_TEXT_DIM);
        return;
    }
    f->rx_peak = follow_peak(f->rx_peak, net->rx_bps);
    f->tx_peak = follow_peak(f->tx_peak, net->tx_bps);
    lv_arc_set_value(f->rx_arc, clamp_pct(net->rx_bps, f->rx_peak));
    lv_arc_set_value(f->tx_arc, clamp_pct(net->tx_bps, f->tx_peak));

    char num[12];
    const char *unit;
    format_rate(net->rx_bps, num, sizeof(num), &unit);
    lv_label_set_text(f->value, num);
    lv_label_set_text_fmt(f->unit, LV_SYMBOL_DOWN " %s", unit);
    set_rate(f->tx, LV_SYMBOL_UP " ", net->tx_bps);
    bool stale = state == METRICS_UI_STALE;
    set_title(&f->title, stale ? COLOR_STALE : COLOR_CYAN, false);
    set_text_color(f->value, stale ? COLOR_TEXT_DIM : COLOR_TEXT);
}

static void update_disk(ui_screen_t *ui, const metrics_disk_t *disk, metrics_ui_state_t state)
{
    ui_disk_t *f = &ui->disk;
    if (state == METRICS_UI_WAITING || !disk->valid) {
        lv_arc_set_value(f->arc, 0);
        lv_label_set_text(f->value, "—");
        lv_label_set_text(f->space, "-- GB");
        lv_obj_add_flag(f->io_row, LV_OBJ_FLAG_HIDDEN);
        set_title(&f->title, placeholder_title(state), false);
        set_text_color(f->value, COLOR_TEXT_DIM);
        return;
    }
    int pct = clamp_pct(disk->used_gb, disk->total_gb);
    bool full = pct >= DISK_FULL_PCT;
    lv_arc_set_value(f->arc, pct);
    set_arc_color(f->arc, full ? COLOR_WARM : COLOR_MEM);
    set_pct(f->value, true, pct);
    if (disk->total_gb >= 1000.0f) {
        lv_label_set_text_fmt(f->space, "%.1f/%.1f TB", disk->used_gb / 1000.0f, disk->total_gb / 1000.0f);
    } else {
        lv_label_set_text_fmt(f->space, "%.0f/%.0f GB", disk->used_gb, disk->total_gb);
    }
    if (disk->has_io) {
        lv_obj_remove_flag(f->io_row, LV_OBJ_FLAG_HIDDEN);
        set_rate(f->read, "", disk->read_bps);
        set_rate(f->write, "", disk->write_bps);
    } else {
        lv_obj_add_flag(f->io_row, LV_OBJ_FLAG_HIDDEN);
    }
    uint32_t tone = state == METRICS_UI_STALE ? COLOR_STALE : full ? COLOR_WARM : COLOR_MEM;
    set_title(&f->title, tone, full);
    set_text_color(f->value, full ? COLOR_WARM : COLOR_TEXT);
}

static void update_battery(ui_screen_t *ui, const metrics_battery_t *bat, metrics_ui_state_t state)
{
    ui_battery_t *f = &ui->battery;
    if (state == METRICS_UI_WAITING || !bat->valid) {
        lv_arc_set_value(f->arc, 0);
        lv_label_set_text(f->value, "—");
        lv_label_set_text(f->status, state == METRICS_UI_WAITING ? "WAITING" : "NO BATTERY");
        set_text_color(f->status, placeholder_title(state));
        lv_label_set_text(f->time, "");
        set_title(&f->title, placeholder_title(state), false);
        set_text_color(f->value, COLOR_TEXT_DIM);
        return;
    }
    int pct = clamp_pct(bat->pct, 100.0f);
    uint32_t color = COLOR_GREEN;
    if (!bat->plugged && pct <= BATTERY_EMPTY_PCT) {
        color = COLOR_HOT;
    } else if (!bat->plugged && pct <= BATTERY_LOW_PCT) {
        color = COLOR_WARM;
    }
    lv_arc_set_value(f->arc, pct);
    set_arc_color(f->arc, color);
    set_pct(f->value, true, pct);
    set_text_color(f->value, color == COLOR_GREEN ? COLOR_TEXT : color);
    const char *status = bat->charging ? LV_SYMBOL_CHARGE " CHARGING" : bat->plugged ? "PLUGGED IN" : "ON BATTERY";
    // The bold font has no symbols: the plain one carries the bolt.
    lv_obj_set_style_text_font(f->status, bat->charging ? &lv_font_montserrat_12 : &lv_font_montserrat_bold_12, 0);
    lv_label_set_text(f->status, status);
    set_text_color(f->status, bat->charging || bat->plugged ? COLOR_GREEN : COLOR_TEXT_DIM);
    if (bat->has_mins && bat->mins > 0 && (bat->charging || !bat->plugged)) {
        const char *what = bat->charging ? "to full" : "left";
        if (bat->mins >= 60) {
            lv_label_set_text_fmt(f->time, "%dh %02dm %s", bat->mins / 60, bat->mins % 60, what);
        } else {
            lv_label_set_text_fmt(f->time, "%dm %s", bat->mins, what);
        }
    } else {
        lv_label_set_text(f->time, "");
    }
    set_title(&f->title, state == METRICS_UI_STALE ? COLOR_STALE : color, color == COLOR_HOT);
}

/* Show frame `index` of our image; false when it can't be had right now. */
static bool image_show_frame(ui_screen_t *ui, int index)
{
    ui_image_face_t *f = &ui->image;
    uint16_t delay = 0;
    if (!media_decode(ui->index, index, f->pixels, &delay)) {
        return false;
    }
    f->frame = index;
    uint32_t now = lv_tick_get();
    f->next_ms = now + (delay > 0 ? delay : 100);
    lv_obj_invalidate(f->img);
    return true;
}

/* Load the image again when one was uploaded or removed since. */
static void update_image(ui_screen_t *ui)
{
    ui_image_face_t *f = &ui->image;
    uint32_t gen = media_generation();
    if (gen == f->generation) {
        return;
    }
    f->generation = gen;
    f->frames = media_frame_count(ui->index);
    if (f->frames > 0 && f->pixels == NULL) {
        f->pixels = heap_caps_malloc(MEDIA_WIDTH * MEDIA_HEIGHT * 2, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
        if (f->pixels == NULL) {
            ESP_LOGE(TAG, "no memory for the image");
            f->frames = 0;
        } else {
            f->dsc = (lv_image_dsc_t) {
                .header = {.magic = LV_IMAGE_HEADER_MAGIC, .cf = LV_COLOR_FORMAT_RGB565, .w = MEDIA_WIDTH,
                           .h = MEDIA_HEIGHT, .stride = MEDIA_WIDTH * 2},
                .data_size = MEDIA_WIDTH * MEDIA_HEIGHT * 2,
                .data = (const uint8_t *) f->pixels,
            };
            lv_image_set_src(f->img, &f->dsc);
        }
    }
    if (f->frames > 0 && image_show_frame(ui, 0)) {
        lv_obj_remove_flag(f->img, LV_OBJ_FLAG_HIDDEN);
        lv_obj_add_flag(f->hint, LV_OBJ_FLAG_HIDDEN);
    } else {
        f->frames = 0;
        lv_obj_add_flag(f->img, LV_OBJ_FLAG_HIDDEN);
        lv_obj_remove_flag(f->hint, LV_OBJ_FLAG_HIDDEN);
        // Try again on the next update (an upload held the slot).
        f->generation = media_present(ui->index) ? UINT32_MAX : gen;
    }
}

/* The next frame of an animation that's on screen, once its delay is up. */
static void image_timer_cb(lv_timer_t *timer)
{
    (void) timer;
    ui_screen_t *screens[] = {&s_cpu, &s_gpu};
    uint32_t now = lv_tick_get();
    for (int i = 0; i < 2; i++) {
        ui_image_face_t *f = &screens[i]->image;
        if (f->frames < 2 || lv_obj_has_flag(f->root, LV_OBJ_FLAG_HIDDEN) || (int32_t) (now - f->next_ms) < 0) {
            continue;
        }
        image_show_frame(screens[i], (f->frame + 1) % f->frames);
    }
}

static void show_face(ui_screen_t *ui, metrics_face_t face)
{
    lv_obj_t *roots[METRICS_FACE_COUNT] = {
        [METRICS_FACE_CLASSIC] = ui->classic.root,
        [METRICS_FACE_RINGS] = ui->rings.root,
        [METRICS_FACE_PLUS] = ui->plus.root,
        [METRICS_FACE_BAR] = ui->bar.root,
        [METRICS_FACE_CLAUDE] = ui->claude.root,
        [METRICS_FACE_CLAWD] = ui->clawd.root,
        [METRICS_FACE_NET] = ui->net.root,
        [METRICS_FACE_DISK] = ui->disk.root,
        [METRICS_FACE_BATTERY] = ui->battery.root,
        [METRICS_FACE_IMAGE] = ui->image.root,
    };
    for (int i = 0; i < METRICS_FACE_COUNT; i++) {
        if (i == (int) face) {
            lv_obj_remove_flag(roots[i], LV_OBJ_FLAG_HIDDEN);
        } else {
            lv_obj_add_flag(roots[i], LV_OBJ_FLAG_HIDDEN);
        }
    }
}

static int fan_rpm_by_id(const metrics_snapshot_t *snap, const char *id);

/* Only the visible face is refreshed; a switch redraws it from the same snapshot. */
static void update_screen(ui_screen_t *ui, metrics_face_t face, metrics_source_t source, const metrics_snapshot_t *snap)
{
    if (face >= METRICS_FACE_COUNT) {
        face = METRICS_FACE_CLASSIC;
    }
    apply_source(ui, source < METRICS_SOURCE_COUNT ? source : METRICS_SOURCE_CPU);
    bool gpu = ui->source == METRICS_SOURCE_GPU;
    const metrics_temp_t *temp = gpu ? &snap->gpu : &snap->cpu;
    float temp_max = gpu ? METRICS_GPU_TEMP_MAX_DEFAULT : METRICS_CPU_TEMP_MAX_DEFAULT;
    int fan_rpm = fan_rpm_by_id(snap, gpu ? "gpu" : "cpu");
    const metrics_claude_t *claude = &snap->claude;
    metrics_ui_state_t state = snap->state;
    switch (face) {
    case METRICS_FACE_NET:
        update_net(ui, &snap->net, state);
        break;
    case METRICS_FACE_DISK:
        update_disk(ui, &snap->disk, state);
        break;
    case METRICS_FACE_BATTERY:
        update_battery(ui, &snap->battery, state);
        break;
    case METRICS_FACE_IMAGE:
        update_image(ui);
        break;
    case METRICS_FACE_RINGS:
        update_rings(ui, temp, temp_max, state);
        break;
    case METRICS_FACE_PLUS:
        update_classic(ui, &ui->plus, temp, state, fan_rpm);
        break;
    case METRICS_FACE_BAR:
        update_classic(ui, &ui->bar, temp, state, fan_rpm);
        break;
    case METRICS_FACE_CLAUDE:
        update_claude(ui, claude, state);
        break;
    case METRICS_FACE_CLAWD:
        update_clawd_face(ui, claude, state);
        break;
    default:
        update_classic(ui, &ui->classic, temp, state, fan_rpm);
        break;
    }
    show_face(ui, face);
}

void ui_watch_create(lv_display_t *disp_cpu, lv_display_t *disp_gpu)
{
    lv_display_set_default(disp_cpu);
    create_screen(&s_cpu, disp_cpu, UI_SCREEN_CPU, METRICS_SOURCE_CPU);

    lv_display_set_default(disp_gpu);
    create_screen(&s_gpu, disp_gpu, UI_SCREEN_GPU, METRICS_SOURCE_GPU);
    lv_timer_create(clawd_timer_cb, CLAWD_TICK_MS, NULL);
    lv_timer_create(image_timer_cb, IMAGE_TICK_MS, NULL);
    ESP_LOGI(TAG, "Watch UI created");
}

static int fan_rpm_by_id(const metrics_snapshot_t *snap, const char *id)
{
    for (size_t i = 0; i < snap->fan_count && i < METRICS_FAN_MAX; i++) {
        if (snap->fans[i].valid && strcmp(snap->fans[i].id, id) == 0) {
            return snap->fans[i].rpm;
        }
    }
    return -1;
}

void ui_watch_update(const metrics_snapshot_t *snap)
{
    if (snap == 0) {
        return;
    }
    update_screen(&s_cpu, snap->cpu_face, snap->cpu_source, snap);
    update_screen(&s_gpu, snap->gpu_face, snap->gpu_source, snap);
}
