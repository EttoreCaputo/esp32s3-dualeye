#include "metrics_model.h"

#include <string.h>

#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"

static SemaphoreHandle_t s_lock;
static metrics_snapshot_t s_snapshot;

static const char *const FACE_NAMES[METRICS_FACE_COUNT] = {
    [METRICS_FACE_CLASSIC] = "classic",
    [METRICS_FACE_RINGS] = "rings",
    [METRICS_FACE_PLUS] = "plus",
    [METRICS_FACE_BAR] = "bar",
    [METRICS_FACE_CLAUDE] = "claude",
    [METRICS_FACE_CLAWD] = "clawd",
    [METRICS_FACE_NET] = "net",
    [METRICS_FACE_DISK] = "disk",
    [METRICS_FACE_BATTERY] = "battery",
    [METRICS_FACE_IMAGE] = "image",
};

static const char *const SOURCE_NAMES[METRICS_SOURCE_COUNT] = {
    [METRICS_SOURCE_CPU] = "cpu",
    [METRICS_SOURCE_GPU] = "gpu",
};

const char *metrics_face_name(metrics_face_t face)
{
    return face < METRICS_FACE_COUNT ? FACE_NAMES[face] : NULL;
}

bool metrics_face_from_name(const char *name, metrics_face_t *out)
{
    for (int i = 0; i < METRICS_FACE_COUNT; i++) {
        if (strcmp(name, FACE_NAMES[i]) == 0) {
            *out = (metrics_face_t) i;
            return true;
        }
    }
    return false;
}

bool metrics_face_has_source(metrics_face_t face)
{
    return face == METRICS_FACE_CLASSIC || face == METRICS_FACE_RINGS || face == METRICS_FACE_PLUS
           || face == METRICS_FACE_BAR;
}

const char *metrics_source_name(metrics_source_t source)
{
    return source < METRICS_SOURCE_COUNT ? SOURCE_NAMES[source] : NULL;
}

bool metrics_source_from_name(const char *name, metrics_source_t *out)
{
    for (int i = 0; i < METRICS_SOURCE_COUNT; i++) {
        if (strcmp(name, SOURCE_NAMES[i]) == 0) {
            *out = (metrics_source_t) i;
            return true;
        }
    }
    return false;
}

void metrics_model_init(void)
{
    s_lock = xSemaphoreCreateMutex();
    memset(&s_snapshot, 0, sizeof(s_snapshot));
    s_snapshot.state = METRICS_UI_WAITING;
}

void metrics_model_get(metrics_snapshot_t *out)
{
    if (out == NULL || s_lock == NULL) {
        return;
    }
    xSemaphoreTake(s_lock, portMAX_DELAY);
    *out = s_snapshot;
    xSemaphoreGive(s_lock);

    if (out->state == METRICS_UI_LIVE && out->updated_ms != 0) {
        uint32_t now_ms = (uint32_t) (esp_timer_get_time() / 1000);
        if ((uint32_t) (now_ms - out->updated_ms) > METRICS_STALE_MS_DEFAULT) {
            out->state = METRICS_UI_STALE;
        }
    }
}

void metrics_model_set(const metrics_snapshot_t *in)
{
    if (in == NULL || s_lock == NULL) {
        return;
    }
    metrics_snapshot_t copy = *in;
    copy.updated_ms = (uint32_t) (esp_timer_get_time() / 1000);
    xSemaphoreTake(s_lock, portMAX_DELAY);
    s_snapshot = copy;
    xSemaphoreGive(s_lock);
}

void metrics_model_load_mock(void)
{
    metrics_snapshot_t mock = {
        .ts = 1710000000,
        .cpu = {.valid = true,
                .temp_c = 62.0f,
                .usage_pct = 47.0f,
                .clock_ghz = 4.8f,
                .power_w = 65.0f,
                .mem_valid = true,
                .mem_used_mb = 12870.0f,
                .mem_total_mb = 31744.0f},
        .gpu = {.valid = true,
                .temp_c = 58.0f,
                .usage_pct = 72.0f,
                .clock_ghz = 2.6f,
                .power_w = 210.0f,
                .mem_valid = true,
                .mem_used_mb = 9216.0f,
                .mem_total_mb = 24576.0f},
        .fan_count = 3,
        .state = METRICS_UI_LIVE,
        .fans =
            {
                {.id = "cpu", .rpm = 1250, .valid = true},
                {.id = "gpu", .rpm = 2100, .valid = true},
                {.id = "sys", .rpm = 900, .valid = true},
            },
    };
    metrics_model_set(&mock);
}
