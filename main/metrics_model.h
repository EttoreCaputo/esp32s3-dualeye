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
 * claude, clawd. */
typedef enum {
    METRICS_FACE_CLASSIC = 0,
    METRICS_FACE_RINGS,
    METRICS_FACE_PLUS,
    METRICS_FACE_BAR,
    METRICS_FACE_CLAUDE,
    METRICS_FACE_CLAWD,
    METRICS_FACE_COUNT,
} metrics_face_t;

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

typedef struct {
    uint32_t ts;
    uint32_t updated_ms;
    metrics_temp_t cpu;
    metrics_temp_t gpu;
    metrics_fan_t fans[METRICS_FAN_MAX];
    size_t fan_count;
    /* Board settings, not from the host's snapshot: filled in before drawing. */
    metrics_face_t cpu_face;
    metrics_face_t gpu_face;
    metrics_claude_t claude;
    metrics_ui_state_t state;
} metrics_snapshot_t;

/** Wire name of a face, or NULL. */
const char *metrics_face_name(metrics_face_t face);
/** Face by wire name; false if there's none. */
bool metrics_face_from_name(const char *name, metrics_face_t *out);

void metrics_model_init(void);
void metrics_model_get(metrics_snapshot_t *out);
void metrics_model_set(const metrics_snapshot_t *in);

/** Phase 1: populate a fixed Watch-style demo snapshot. */
void metrics_model_load_mock(void);

#ifdef __cplusplus
}
#endif
