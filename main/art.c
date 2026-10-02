#include "art.h"

#include <string.h>

#include "esp_heap_caps.h"
#include "esp_log.h"
#include "lvgl_port.h"
#include "mbedtls/base64.h"

static const char *TAG = "art";

/* What's on screen, and where the next cover arrives. */
static uint16_t *s_front;
static uint8_t *s_back;
static lv_image_dsc_t s_dsc;
static volatile uint32_t s_id;
static volatile uint32_t s_generation;
/* The cover arriving: its id and how much of it is in. */
static uint32_t s_incoming;
static size_t s_received;

void art_init(void)
{
    s_front = heap_caps_calloc(1, ART_BYTES, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    s_back = heap_caps_malloc(ART_BYTES, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    if (s_front == NULL || s_back == NULL) {
        ESP_LOGE(TAG, "no memory for cover art");
        heap_caps_free(s_front);
        heap_caps_free(s_back);
        s_front = NULL;
        s_back = NULL;
        return;
    }
    s_dsc = (lv_image_dsc_t) {
        .header = {.magic = LV_IMAGE_HEADER_MAGIC, .cf = LV_COLOR_FORMAT_RGB565, .w = ART_SIZE, .h = ART_SIZE,
                   .stride = ART_SIZE * 2},
        .data_size = ART_BYTES,
        .data = (const uint8_t *) s_front,
    };
}

uint32_t art_id(void)
{
    return s_id;
}

const lv_image_dsc_t *art_image(void)
{
    return s_front != NULL ? &s_dsc : NULL;
}

uint32_t art_generation(void)
{
    return s_generation;
}

static cJSON *write_chunk(const cJSON *params, const char **message)
{
    static uint8_t buf[ART_CHUNK];
    const cJSON *id = cJSON_GetObjectItemCaseSensitive(params, "id");
    const cJSON *offset = cJSON_GetObjectItemCaseSensitive(params, "offset");
    const cJSON *data = cJSON_GetObjectItemCaseSensitive(params, "data");
    if (!cJSON_IsNumber(id) || id->valuedouble < 1 || !cJSON_IsNumber(offset) || !cJSON_IsString(data)) {
        *message = "expected {\"id\": n > 0, \"offset\": bytes, \"data\": base64}";
        return NULL;
    }
    if (s_back == NULL) {
        *message = "no memory for cover art";
        return NULL;
    }
    uint32_t art = (uint32_t) id->valuedouble;
    size_t at = (size_t) offset->valuedouble;
    if (at == 0) {
        s_incoming = art;
        s_received = 0;
    }
    if (art != s_incoming || at != s_received) {
        *message = "chunks must come in order, from offset 0";
        return NULL;
    }
    const char *b64 = data->valuestring;
    size_t len = 0;
    if (mbedtls_base64_decode(buf, sizeof(buf), &len, (const unsigned char *) b64, strlen(b64)) != 0
        || s_received + len > ART_BYTES) {
        *message = "data isn't base64, or is over the chunk size or the picture";
        return NULL;
    }
    memcpy(s_back + s_received, buf, len);
    s_received += len;

    cJSON *result = cJSON_CreateObject();
    cJSON_AddNumberToObject(result, "received", (double) s_received);
    if (s_received == ART_BYTES) {
        // In place, under the lock: no frame is drawn from half a cover.
        lvgl_port_lock();
        memcpy(s_front, s_back, ART_BYTES);
        s_id = art;
        s_generation++;
        lvgl_port_unlock();
        s_incoming = 0;
        s_received = 0;
        cJSON_AddBoolToObject(result, "done", true);
    }
    return result;
}

cJSON *art_rpc(const char *method, const cJSON *params, const char **message)
{
    if (strcmp(method, "music/art") == 0) {
        return write_chunk(params, message);
    }
    if (strcmp(method, "music/info") == 0) {
        cJSON *result = cJSON_CreateObject();
        cJSON_AddNumberToObject(result, "size", ART_SIZE);
        cJSON_AddNumberToObject(result, "chunk", ART_CHUNK);
        cJSON_AddNumberToObject(result, "art", (double) s_id);
        return result;
    }
    *message = "method not found";
    return NULL;
}
