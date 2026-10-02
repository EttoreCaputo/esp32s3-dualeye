#include "board_tools.h"

#include <stdio.h>
#include <string.h>

#include "board_settings.h"
#include "esp_app_desc.h"
#include "esp_heap_caps.h"
#include "esp_timer.h"
#include "link.h"
#include "lvgl_port.h"
#include "metrics_model.h"
#include "playback.h"
#include "ui_eyes.h"
#include "media.h"
#include "ui_toast.h"
#include "ui_voice.h"
#include "voice.h"

#define TEXT_MAX 160

#define SCREEN_PROP \
    "\"screen\":{\"type\":\"string\",\"enum\":[\"left\",\"right\",\"both\"],\"default\":\"both\"," \
    "\"description\":\"left or right round screen, or both\"}"

/** A tool fills `text` with what it did, or why it refused (returning false). */
typedef bool (*tool_fn_t)(const cJSON *args, char *text, cJSON **structured);

typedef struct {
    const char *name;
    const char *description;
    const char *schema;
    tool_fn_t fn;
} tool_t;

static const char *const SCREEN_NAMES[BOARD_LCD_COUNT] = {[UI_SCREEN_CPU] = "left", [UI_SCREEN_GPU] = "right"};

/** Which screens `screen` names; both when it's missing. */
static bool get_screens(const cJSON *args, bool on[BOARD_LCD_COUNT], const char **label, char *text)
{
    const cJSON *screen = cJSON_GetObjectItemCaseSensitive(args, "screen");
    const char *name = cJSON_IsString(screen) ? screen->valuestring : "both";
    if (screen != NULL && !cJSON_IsString(screen)) {
        name = "";
    }
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        on[i] = strcmp(name, "both") == 0 || strcmp(name, SCREEN_NAMES[i]) == 0;
    }
    if (!on[0] && !on[1]) {
        snprintf(text, TEXT_MAX, "screen must be left, right or both");
        return false;
    }
    *label = strcmp(name, "both") == 0 ? "both screens" : name;
    return true;
}

static bool get_int(const cJSON *args, const char *key, int *out, char *text)
{
    const cJSON *v = cJSON_GetObjectItemCaseSensitive(args, key);
    if (!cJSON_IsNumber(v) || v->valuedouble != (double) (int) v->valuedouble) {
        snprintf(text, TEXT_MAX, "%s must be an integer", key);
        return false;
    }
    *out = (int) v->valuedouble;
    return true;
}

static bool tool_set_face(const cJSON *args, char *text, cJSON **structured)
{
    bool on[BOARD_LCD_COUNT];
    const char *label = NULL;
    if (!get_screens(args, on, &label, text)) {
        return false;
    }
    const cJSON *face = cJSON_GetObjectItemCaseSensitive(args, "face");
    metrics_face_t f;
    if (!cJSON_IsString(face) || !metrics_face_from_name(face->valuestring, &f)) {
        snprintf(text, TEXT_MAX,
                 "face must be one of classic, rings, plus, bar, claude, clawd, net, disk, battery, image, timer");
        return false;
    }
    const cJSON *source = cJSON_GetObjectItemCaseSensitive(args, "source");
    metrics_source_t src = METRICS_SOURCE_CPU;
    bool has_source = source != NULL;
    if (has_source && (!cJSON_IsString(source) || !metrics_source_from_name(source->valuestring, &src))) {
        snprintf(text, TEXT_MAX, "source must be cpu or gpu");
        return false;
    }
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        if (on[i]) {
            board_settings_set_face(i, f);
            if (has_source) {
                board_settings_set_source(i, src);
            }
        }
    }
    if (has_source && metrics_face_has_source(f)) {
        snprintf(text, TEXT_MAX, "%s: %s with the %s", label, metrics_face_name(f), metrics_source_name(src));
    } else {
        snprintf(text, TEXT_MAX, "%s: %s", label, metrics_face_name(f));
    }
    return true;
}

static bool tool_set_rotation(const cJSON *args, char *text, cJSON **structured)
{
    bool on[BOARD_LCD_COUNT];
    const char *label = NULL;
    int deg = 0;
    if (!get_screens(args, on, &label, text) || !get_int(args, "degrees", &deg, text)) {
        return false;
    }
    if (deg != 0 && deg != 90 && deg != 180 && deg != 270) {
        snprintf(text, TEXT_MAX, "degrees must be 0, 90, 180 or 270");
        return false;
    }
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        if (on[i]) {
            board_settings_set_rotation(i, (uint16_t) deg);
        }
    }
    snprintf(text, TEXT_MAX, "%s: turned %d degrees", label, deg);
    return true;
}

static bool tool_set_brightness(const cJSON *args, char *text, cJSON **structured)
{
    bool on[BOARD_LCD_COUNT];
    const char *label = NULL;
    int pct = 0;
    if (!get_screens(args, on, &label, text) || !get_int(args, "percent", &pct, text)) {
        return false;
    }
    if (pct < 0 || pct > 100) {
        snprintf(text, TEXT_MAX, "percent must be 0 to 100");
        return false;
    }
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        if (on[i] && board_settings_set_brightness(i, (uint8_t) pct) != ESP_OK) {
            snprintf(text, TEXT_MAX, "backlight of the %s screen failed", SCREEN_NAMES[i]);
            return false;
        }
    }
    snprintf(text, TEXT_MAX, "%s: brightness %d%%", label, pct);
    return true;
}

static bool tool_show_text(const cJSON *args, char *text, cJSON **structured)
{
    bool on[BOARD_LCD_COUNT];
    const char *label = NULL;
    if (!get_screens(args, on, &label, text)) {
        return false;
    }
    const cJSON *msg = cJSON_GetObjectItemCaseSensitive(args, "text");
    if (!cJSON_IsString(msg) || msg->valuestring[0] == '\0') {
        snprintf(text, TEXT_MAX, "text must be a non-empty string");
        return false;
    }
    int seconds = 4;
    if (cJSON_GetObjectItemCaseSensitive(args, "seconds") != NULL
        && (!get_int(args, "seconds", &seconds, text) || seconds < 1 || seconds > 30)) {
        snprintf(text, TEXT_MAX, "seconds must be 1 to 30");
        return false;
    }
    lvgl_port_lock();
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        if (on[i]) {
            ui_toast_show(i, msg->valuestring, (uint32_t) seconds * 1000);
        }
    }
    lvgl_port_unlock();
    snprintf(text, TEXT_MAX, "%s: showing it for %d s", label, seconds);
    return true;
}

static bool tool_set_mic(const cJSON *args, char *text, cJSON **structured)
{
    const cJSON *muted = cJSON_GetObjectItemCaseSensitive(args, "muted");
    if (!cJSON_IsBool(muted)) {
        snprintf(text, TEXT_MAX, "muted must be true or false");
        return false;
    }
    if (!voice_available()) {
        snprintf(text, TEXT_MAX, "voice is not available on this board");
        return false;
    }
    bool on = cJSON_IsTrue(muted);
    board_settings_set_mic_muted(on);
    voice_set_muted(on);
    snprintf(text, TEXT_MAX, on ? "microphone muted: not listening for \"%s\"" : "listening for \"%s\"",
             voice_wake_word());
    return true;
}

static bool tool_set_wake_word(const cJSON *args, char *text, cJSON **structured)
{
    const cJSON *word = cJSON_GetObjectItemCaseSensitive(args, "word");
    if (!cJSON_IsString(word)) {
        snprintf(text, TEXT_MAX, "word must be a string");
        return false;
    }
    if (!voice_available()) {
        snprintf(text, TEXT_MAX, "voice is not available on this board");
        return false;
    }
    esp_err_t err = voice_set_wake_word(word->valuestring);
    if (err == ESP_ERR_NOT_FOUND) {
        snprintf(text, TEXT_MAX, "no model for \"%s\" on this board", word->valuestring);
        return false;
    }
    if (err != ESP_OK) {
        snprintf(text, TEXT_MAX, "switching failed, still listening for \"%s\"", voice_wake_word());
        return false;
    }
    board_settings_set_wake_word(voice_wake_word_id());
    snprintf(text, TEXT_MAX, "wake word is now \"%s\"", voice_wake_word());
    return true;
}

static bool tool_set_volume(const cJSON *args, char *text, cJSON **structured)
{
    int pct = 0;
    if (!get_int(args, "percent", &pct, text)) {
        return false;
    }
    if (pct < 0 || pct > 100) {
        snprintf(text, TEXT_MAX, "percent must be 0 to 100");
        return false;
    }
    if (!playback_available() || board_settings_set_volume((uint8_t) pct) != ESP_OK) {
        snprintf(text, TEXT_MAX, "the speaker is not available on this board");
        return false;
    }
    snprintf(text, TEXT_MAX, "speaker volume %d%%", pct);
    return true;
}

static bool tool_set_eyes(const cJSON *args, char *text, cJSON **structured)
{
    const cJSON *on = cJSON_GetObjectItemCaseSensitive(args, "on");
    const cJSON *idle = cJSON_GetObjectItemCaseSensitive(args, "idle");
    if ((on != NULL && !cJSON_IsBool(on)) || (idle != NULL && !cJSON_IsBool(idle)) || (on == NULL && idle == NULL)) {
        snprintf(text, TEXT_MAX, "give on and/or idle, true or false");
        return false;
    }
    int n = 0;
    lvgl_port_lock();
    if (on != NULL) {
        bool eyes = cJSON_IsTrue(on);
        board_settings_set_eyes(eyes);
        ui_voice_set_eyes(eyes);
        n += snprintf(text, TEXT_MAX, eyes ? "talking shows animated eyes" : "talking shows a ring round the screens");
    }
    if (idle != NULL) {
        bool scenes = cJSON_IsTrue(idle);
        board_settings_set_idle_eyes(scenes);
        ui_eyes_set_idle(scenes);
        snprintf(text + n, TEXT_MAX - n, "%s%s", n ? "; " : "",
                 scenes ? "the eyes play a scene now and then" : "no eye scenes while idle");
    }
    lvgl_port_unlock();
    return true;
}

static bool tool_play_eyes(const cJSON *args, char *text, cJSON **structured)
{
    const cJSON *name = cJSON_GetObjectItemCaseSensitive(args, "name");
    if (name != NULL && !cJSON_IsString(name)) {
        snprintf(text, TEXT_MAX, "name must be a string");
        return false;
    }
    const char *which = name != NULL ? name->valuestring : NULL;
    lvgl_port_lock();
    bool ok = ui_eyes_play(which);
    lvgl_port_unlock();
    if (ok) {
        snprintf(text, TEXT_MAX, "playing %s", which != NULL ? which : "a scene");
        return true;
    }
    if (voice_state() != VOICE_IDLE) {
        snprintf(text, TEXT_MAX, "not now: a conversation is on");
        return false;
    }
    const char *names[24];
    int count = ui_eyes_animations(names, 24);
    int len = snprintf(text, TEXT_MAX, "unknown scene; one of:");
    for (int i = 0; i < count && len < TEXT_MAX; i++) {
        len += snprintf(text + len, TEXT_MAX - len, " %s", names[i]);
    }
    return false;
}

static const char *metrics_state_name(metrics_ui_state_t state)
{
    switch (state) {
    case METRICS_UI_LIVE:
        return "live";
    case METRICS_UI_STALE:
        return "stale";
    case METRICS_UI_ERROR:
        return "error";
    default:
        return "waiting";
    }
}

static bool tool_get_state(const cJSON *args, char *text, cJSON **structured)
{
    board_settings_t settings;
    board_settings_get(&settings);
    metrics_snapshot_t snap;
    metrics_model_get(&snap);
    link_stats_t link;
    link_get_stats(&link);

    cJSON *st = cJSON_CreateObject();
    cJSON_AddStringToObject(st, "firmware", esp_app_get_description()->version);
    cJSON_AddNumberToObject(st, "uptime_s", (double) (esp_timer_get_time() / 1000000));
    cJSON_AddStringToObject(st, "metrics", metrics_state_name(snap.state));
    cJSON *screens = cJSON_AddObjectToObject(st, "screens");
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        cJSON *s = cJSON_AddObjectToObject(screens, SCREEN_NAMES[i]);
        cJSON_AddStringToObject(s, "face", metrics_face_name(settings.face[i]));
        cJSON_AddStringToObject(s, "source", metrics_source_name(settings.source[i]));
        cJSON_AddBoolToObject(s, "image", media_present(i));
        cJSON_AddNumberToObject(s, "rotation", settings.rot[i]);
        cJSON_AddNumberToObject(s, "brightness", settings.brightness[i]);
    }
    cJSON *voice = cJSON_AddObjectToObject(st, "voice");
    cJSON_AddBoolToObject(voice, "available", voice_available());
    cJSON_AddBoolToObject(voice, "eyes", settings.eyes);
    cJSON_AddBoolToObject(voice, "idle_eyes", settings.idle_eyes);
    if (voice_available()) {
        cJSON_AddStringToObject(voice, "wake_word", voice_wake_word());
        cJSON_AddStringToObject(voice, "wake_word_id", voice_wake_word_id());
        cJSON_AddStringToObject(voice, "model", voice_wake_model());
        const char *ids[8];
        int n = voice_wake_words(ids, 8);
        cJSON_AddItemToObject(voice, "wake_words", cJSON_CreateStringArray(ids, n));
        cJSON_AddBoolToObject(voice, "muted", voice_muted());
        cJSON_AddStringToObject(voice, "state", voice_state_name(voice_state()));
    }
    cJSON *audio = cJSON_AddObjectToObject(st, "audio");
    cJSON_AddBoolToObject(audio, "speaker", playback_available());
    cJSON_AddNumberToObject(audio, "volume", settings.volume);
    cJSON_AddBoolToObject(audio, "playing", playback_active());
    lvgl_port_stats_t ui;
    lvgl_port_get_stats(&ui);
    cJSON *u = cJSON_AddObjectToObject(st, "ui");
    cJSON_AddNumberToObject(u, "busy_pct", (int) (ui.busy_pct * 10.0f + 0.5f) / 10.0);
    cJSON_AddNumberToObject(u, "max_frame_ms", (int) (ui.max_us / 100) / 10.0);
    cJSON *mem = cJSON_AddObjectToObject(st, "memory");
    cJSON_AddNumberToObject(mem, "internal_free", heap_caps_get_free_size(MALLOC_CAP_INTERNAL));
    cJSON_AddNumberToObject(mem, "internal_min", heap_caps_get_minimum_free_size(MALLOC_CAP_INTERNAL));
    cJSON_AddNumberToObject(mem, "psram_free", heap_caps_get_free_size(MALLOC_CAP_SPIRAM));
    cJSON *l = cJSON_AddObjectToObject(st, "link");
    cJSON_AddNumberToObject(l, "rx_frames", link.rx_frames);
    cJSON_AddNumberToObject(l, "rx_bad", link.rx_bad);
    cJSON_AddNumberToObject(l, "tx_frames", link.tx_frames);
    cJSON_AddNumberToObject(l, "tx_dropped", link.tx_dropped);
    *structured = st;
    // The text is filled in by the caller from `structured`.
    text[0] = '\0';
    return true;
}

static const tool_t TOOLS[] = {
    {
        .name = "set_face",
        .description = "Switch the watch face of one or both round screens. Any face goes on either screen.",
        .schema = "{\"type\":\"object\",\"properties\":{"
                  "\"face\":{\"type\":\"string\",\"enum\":[\"classic\",\"rings\",\"plus\",\"bar\",\"claude\",\"clawd\","
                  "\"net\",\"disk\",\"battery\",\"image\",\"timer\"],"
                  "\"description\":\"classic: temperature, clock, power, load ring and fan; rings: load, temperature "
                  "and memory rings; plus: classic with a memory bar and numbers; bar: classic with a small memory "
                  "bar; claude: Claude Code usage limits and tokens; clawd: animated Claude Code mascot; net: "
                  "download and upload speed; disk: system disk space and activity; battery: the laptop's battery; "
                  "image: the picture or GIF uploaded for that screen; timer: the host's timers and reminders "
                  "counting down (a running timer also shows by itself)\"},"
                  "\"source\":{\"type\":\"string\",\"enum\":[\"cpu\",\"gpu\"],\"description\":\"Whose metrics "
                  "classic, rings, plus and bar show; unchanged when left out\"},"
                  SCREEN_PROP "},\"required\":[\"face\"]}",
        .fn = tool_set_face,
    },
    {
        .name = "set_rotation",
        .description = "Turn one or both screens clockwise, for a board that sits another way round.",
        .schema = "{\"type\":\"object\",\"properties\":{"
                  "\"degrees\":{\"type\":\"integer\",\"enum\":[0,90,180,270]}," SCREEN_PROP
                  "},\"required\":[\"degrees\"]}",
        .fn = tool_set_rotation,
    },
    {
        .name = "set_brightness",
        .description = "Set the backlight of one or both screens; 0 turns it off.",
        .schema = "{\"type\":\"object\",\"properties\":{"
                  "\"percent\":{\"type\":\"integer\",\"minimum\":0,\"maximum\":100}," SCREEN_PROP
                  "},\"required\":[\"percent\"]}",
        .fn = tool_set_brightness,
    },
    {
        .name = "show_text",
        .description = "Show a short message over the watch face for a few seconds.",
        .schema = "{\"type\":\"object\",\"properties\":{"
                  "\"text\":{\"type\":\"string\",\"maxLength\":120,\"description\":\"Letters without accents show "
                  "best; accented ones lose the accent\"},"
                  "\"seconds\":{\"type\":\"integer\",\"minimum\":1,\"maximum\":30,\"default\":4}," SCREEN_PROP
                  "},\"required\":[\"text\"]}",
        .fn = tool_show_text,
    },
    {
        .name = "set_mic",
        .description = "Mute or unmute the microphone. Muted, the board doesn't listen for its wake word.",
        .schema = "{\"type\":\"object\",\"properties\":{\"muted\":{\"type\":\"boolean\"}},\"required\":[\"muted\"]}",
        .fn = tool_set_mic,
    },
    {
        .name = "set_wake_word",
        .description = "Choose the word the board listens for before a voice command. The board remembers it.",
        .schema = "{\"type\":\"object\",\"properties\":{\"word\":{\"type\":\"string\","
                  "\"enum\":[\"alexa\",\"hiesp\"],\"description\":\"alexa: say Alexa (default); hiesp: say Hi ESP\"}},"
                  "\"required\":[\"word\"]}",
        .fn = tool_set_wake_word,
    },
    {
        .name = "set_volume",
        .description = "Set the speaker volume the board talks with; 0 is silent. The board remembers it.",
        .schema = "{\"type\":\"object\",\"properties\":{"
                  "\"percent\":{\"type\":\"integer\",\"minimum\":0,\"maximum\":100}},\"required\":[\"percent\"]}",
        .fn = tool_set_volume,
    },
    {
        .name = "set_eyes",
        .description = "Choose what the screens show during a voice conversation: animated eyes (on, the default) "
                       "or a coloured ring round the watch face (off); and whether the eyes play a short scene "
                       "now and then while nobody is talking (idle, on by default). The board remembers both.",
        .schema = "{\"type\":\"object\",\"properties\":{\"on\":{\"type\":\"boolean\"},"
                  "\"idle\":{\"type\":\"boolean\"}},\"minProperties\":1}",
        .fn = tool_set_eyes,
    },
    {
        .name = "play_eyes",
        .description = "Play one of the eyes' short scenes on the screens now (a few seconds), or a random one "
                       "without a name. Refused during a voice conversation.",
        // The names of SKITS in ui_eyes.c.
        .schema = "{\"type\":\"object\",\"properties\":{\"name\":{\"type\":\"string\",\"enum\":["
                  "\"look_around\",\"sleepy\",\"suspicious\",\"happy\",\"surprised\",\"wink\",\"angry\","
                  "\"sad\",\"dizzy\",\"cross_eyed\",\"eye_roll\",\"curious\",\"love\",\"scan\",\"shy\","
                  "\"flutter\"]}}}",
        .fn = tool_play_eyes,
    },
    {
        .name = "get_state",
        .description = "Read the board's state: firmware, uptime, whether metrics are live, each screen's face, "
                       "rotation and brightness, the voice state (wake word, muted, listening), the speaker volume and how busy the UI is.",
        .schema = "{\"type\":\"object\",\"properties\":{}}",
        .fn = tool_get_state,
    },
};

#define TOOL_COUNT (sizeof(TOOLS) / sizeof(TOOLS[0]))

cJSON *board_tools_list(void)
{
    cJSON *result = cJSON_CreateObject();
    cJSON *list = cJSON_AddArrayToObject(result, "tools");
    for (size_t i = 0; i < TOOL_COUNT; i++) {
        cJSON *t = cJSON_CreateObject();
        cJSON_AddStringToObject(t, "name", TOOLS[i].name);
        cJSON_AddStringToObject(t, "description", TOOLS[i].description);
        cJSON_AddItemToObject(t, "inputSchema", cJSON_Parse(TOOLS[i].schema));
        cJSON_AddItemToArray(list, t);
    }
    return result;
}

cJSON *board_tools_call(const char *name, const cJSON *args)
{
    const tool_t *tool = NULL;
    for (size_t i = 0; i < TOOL_COUNT && tool == NULL; i++) {
        if (strcmp(TOOLS[i].name, name) == 0) {
            tool = &TOOLS[i];
        }
    }
    if (tool == NULL) {
        return NULL;
    }
    char text[TEXT_MAX];
    cJSON *structured = NULL;
    bool ok = tool->fn(args, text, &structured);

    cJSON *result = cJSON_CreateObject();
    cJSON *content = cJSON_AddArrayToObject(result, "content");
    cJSON *item = cJSON_CreateObject();
    cJSON_AddStringToObject(item, "type", "text");
    if (structured != NULL && text[0] == '\0') {
        char *json = cJSON_PrintUnformatted(structured);
        cJSON_AddStringToObject(item, "text", json != NULL ? json : "");
        cJSON_free(json);
    } else {
        cJSON_AddStringToObject(item, "text", text);
    }
    cJSON_AddItemToArray(content, item);
    if (structured != NULL) {
        cJSON_AddItemToObject(result, "structuredContent", structured);
    }
    cJSON_AddBoolToObject(result, "isError", !ok);
    return result;
}
