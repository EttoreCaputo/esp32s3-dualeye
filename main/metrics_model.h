#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define METRICS_FAN_MAX 3
#define METRICS_CPU_TEMP_MAX_DEFAULT 100.0f
#define METRICS_GPU_TEMP_MAX_DEFAULT 100.0f
#define METRICS_FAN_RPM_MAX_DEFAULT 3000
#define METRICS_STALE_MS_DEFAULT 3000

typedef enum {
    METRICS_UI_WAITING = 0,
    METRICS_UI_LIVE,
    METRICS_UI_STALE,
    METRICS_UI_ERROR,
} metrics_ui_state_t;

/** Watch face of one screen. Names on the wire: classic, rings, plus, bar,
 * claude, clawd, net, disk, battery, image, timer. New ones go at the end:
 * NVS keeps the number. */
typedef enum {
    METRICS_FACE_CLASSIC = 0,
    METRICS_FACE_RINGS,
    METRICS_FACE_PLUS,
    METRICS_FACE_BAR,
    METRICS_FACE_CLAUDE,
    METRICS_FACE_CLAWD,
    METRICS_FACE_NET,
    METRICS_FACE_DISK,
    METRICS_FACE_BATTERY,
    METRICS_FACE_IMAGE,
    METRICS_FACE_TIMER,
    METRICS_FACE_COUNT,
} metrics_face_t;

/** Whose metrics classic, rings, plus and bar show. Names on the wire: cpu,
 * gpu. By default the left screen shows the CPU, the right one the GPU. */
typedef enum {
    METRICS_SOURCE_CPU = 0,
    METRICS_SOURCE_GPU,
    METRICS_SOURCE_COUNT,
} metrics_source_t;

/** What Claude Code is doing. Names on the wire: sleep, work, idle. */
typedef enum {
    METRICS_CLAUDE_SLEEP = 0,
    METRICS_CLAUDE_WORK,
    METRICS_CLAUDE_IDLE,
} metrics_claude_state_t;

/* Claude Code usage for the claude and clawd faces; the has_* flags mark the
 * optional fields the host sent. */
typedef struct {
    bool valid;
    float tokens;
    float today;
    bool has_left;
    int left_min;
    bool has_session;
    float session_pct;
    bool has_week;
    float week_pct;
    metrics_claude_state_t state;
    char model[16];
} metrics_claude_t;

typedef struct {
    bool valid;
    float temp_c;
    float usage_pct;
    float clock_ghz;
    float power_w;
    /* System RAM for the CPU, VRAM for the GPU. */
    bool mem_valid;
    float mem_used_mb;
    float mem_total_mb;
} metrics_temp_t;

typedef struct {
    char id[12];
    int rpm;
    bool valid;
} metrics_fan_t;

/* Network throughput, all interfaces but loopback, bytes per second. */
typedef struct {
    bool valid;
    float rx_bps;
    float tx_bps;
} metrics_net_t;

/* The system disk: space and throughput (bytes per second, when the host
 * can tell). */
typedef struct {
    bool valid;
    float used_gb;
    float total_gb;
    bool has_io;
    float read_bps;
    float write_bps;
} metrics_disk_t;

/* The laptop's battery; not valid on a desktop. */
typedef struct {
    bool valid;
    float pct;
    bool charging;
    /* On mains power: charging, or full. */
    bool plugged;
    /* Minutes to empty (on battery) or to full (charging). */
    bool has_mins;
    int mins;
} metrics_battery_t;

/** What a timer counts down to. Names on the wire: timer, work, break,
 * reminder (work and break are a pomodoro's). */
typedef enum {
    METRICS_TIMER_PLAIN = 0,
    METRICS_TIMER_WORK,
    METRICS_TIMER_BREAK,
    METRICS_TIMER_REMINDER,
} metrics_timer_kind_t;

/* The host's timer that ends first (or is ringing), for the timer face. The
 * host keeps the timers; the board counts down from `left_s` between
 * snapshots and rings while `ringing`. */
typedef struct {
    bool valid;
    metrics_timer_kind_t kind;
    bool paused;
    bool ringing;
    float left_s;
    float total_s;
    /* Other timers running besides this one. */
    int more;
    /* A pomodoro's round and how many there are; 0 for other timers. */
    int round;
    int rounds;
    /* The screen it takes over while it runs (UI_SCREEN_*), -1 for none. */
    int screen;
    char label[28];
} metrics_timer_t;

typedef struct {
    uint32_t ts;
    uint32_t updated_ms;
    metrics_temp_t cpu;
    metrics_temp_t gpu;
    metrics_fan_t fans[METRICS_FAN_MAX];
    size_t fan_count;
    metrics_net_t net;
    metrics_disk_t disk;
    metrics_battery_t battery;
    metrics_timer_t timer;
    /* Board settings, not from the host's snapshot: filled in before drawing. */
    metrics_face_t cpu_face;
    metrics_face_t gpu_face;
    metrics_source_t cpu_source;
    metrics_source_t gpu_source;
    metrics_claude_t claude;
    metrics_ui_state_t state;
} metrics_snapshot_t;

/** Wire name of a face, or NULL. */
const char *metrics_face_name(metrics_face_t face);
/** Face by wire name; false if there's none. */
bool metrics_face_from_name(const char *name, metrics_face_t *out);
/** Whether `face` shows a CPU's or GPU's metrics, so it has a source. */
bool metrics_face_has_source(metrics_face_t face);
const char *metrics_source_name(metrics_source_t source);
bool metrics_source_from_name(const char *name, metrics_source_t *out);

void metrics_model_init(void);
void metrics_model_get(metrics_snapshot_t *out);
void metrics_model_set(const metrics_snapshot_t *in);

/** Phase 1: populate a fixed Watch-style demo snapshot. */
void metrics_model_load_mock(void);

#ifdef __cplusplus
}
#endif
