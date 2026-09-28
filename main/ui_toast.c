#include "ui_toast.h"

#include <string.h>

#define TOAST_MAX_W 188
#define TOAST_BG 0x1C1C1E
#define TOAST_BORDER 0x3A3A3C

typedef struct {
    lv_obj_t *box;
    lv_obj_t *label;
    lv_timer_t *timer;
} ui_toast_t;

static ui_toast_t s_toast[BOARD_LCD_COUNT];

static void hide_cb(lv_timer_t *timer)
{
    ui_toast_t *t = lv_timer_get_user_data(timer);
    lv_obj_add_flag(t->box, LV_OBJ_FLAG_HIDDEN);
    lv_timer_pause(timer);
}

void ui_toast_create(lv_display_t *const displays[BOARD_LCD_COUNT])
{
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        ui_toast_t *t = &s_toast[i];
        t->box = lv_obj_create(lv_display_get_layer_top(displays[i]));
        lv_obj_remove_style_all(t->box);
        lv_obj_set_size(t->box, LV_SIZE_CONTENT, LV_SIZE_CONTENT);
        lv_obj_set_style_max_width(t->box, TOAST_MAX_W, 0);
        lv_obj_set_style_bg_color(t->box, lv_color_hex(TOAST_BG), 0);
        lv_obj_set_style_bg_opa(t->box, LV_OPA_90, 0);
        lv_obj_set_style_border_color(t->box, lv_color_hex(TOAST_BORDER), 0);
        lv_obj_set_style_border_width(t->box, 1, 0);
        lv_obj_set_style_radius(t->box, 14, 0);
        lv_obj_set_style_pad_hor(t->box, 14, 0);
        lv_obj_set_style_pad_ver(t->box, 10, 0);
        lv_obj_remove_flag(t->box, LV_OBJ_FLAG_SCROLLABLE);
        lv_obj_center(t->box);

        t->label = lv_label_create(t->box);
        lv_obj_set_style_text_font(t->label, &lv_font_montserrat_16, 0);
        lv_obj_set_style_text_color(t->label, lv_color_white(), 0);
        lv_obj_set_style_text_align(t->label, LV_TEXT_ALIGN_CENTER, 0);
        lv_obj_set_style_max_width(t->label, TOAST_MAX_W - 28, 0);
        lv_obj_set_width(t->label, LV_SIZE_CONTENT);
        lv_label_set_long_mode(t->label, LV_LABEL_LONG_WRAP);

        t->timer = lv_timer_create(hide_cb, 1000, t);
        lv_timer_pause(t->timer);
        lv_obj_add_flag(t->box, LV_OBJ_FLAG_HIDDEN);
    }
}

/* U+00C0..U+00FF without their accents: the built-in fonts are ASCII only,
 * and Italian needs à, è, é, ì, ò, ù. */
static const char LATIN1_BASE[] = "AAAAAAACEEEEIIIIDNOOOOOxOUUUUYTs"
                                  "aaaaaaaceeeeiiiidnooooo/ouuuuyty";

/** Copy UTF-8 `in` into ASCII `out`. */
static void to_ascii(const char *in, char *out, size_t out_len)
{
    size_t n = 0;
    const unsigned char *p = (const unsigned char *) in;
    while (*p != '\0' && n + 1 < out_len) {
        unsigned c = *p;
        if (c < 0x80) {
            out[n++] = (c >= 0x20 || c == '\n') ? (char) c : ' ';
            p++;
            continue;
        }
        // Length of the sequence from its lead byte; skip stray continuations.
        int len = (c & 0xE0) == 0xC0 ? 2 : (c & 0xF0) == 0xE0 ? 3 : (c & 0xF8) == 0xF0 ? 4 : 1;
        unsigned cp = 0;
        if (len == 2 && (p[1] & 0xC0) == 0x80) {
            cp = ((c & 0x1F) << 6) | (p[1] & 0x3F);
        }
        out[n++] = (cp >= 0xC0 && cp <= 0xFF) ? LATIN1_BASE[cp - 0xC0] : '?';
        for (int i = 0; i < len && *p != '\0'; i++) {
            p++;
        }
    }
    out[n] = '\0';
}

void ui_toast_show(int screen, const char *text, uint32_t ms)
{
    if (screen < 0 || screen >= BOARD_LCD_COUNT) {
        return;
    }
    ui_toast_t *t = &s_toast[screen];
    char ascii[UI_TOAST_TEXT_MAX + 1];
    to_ascii(text, ascii, sizeof(ascii));
    lv_label_set_text(t->label, ascii);
    lv_obj_center(t->box);
    lv_obj_remove_flag(t->box, LV_OBJ_FLAG_HIDDEN);
    lv_timer_set_period(t->timer, ms);
    lv_timer_reset(t->timer);
    lv_timer_resume(t->timer);
}
