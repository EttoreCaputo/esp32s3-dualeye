#include "media.h"

#include <stdio.h>
#include <string.h>

#include "board_display.h"
#include "esp_log.h"
#include "esp_partition.h"
#include "esp_rom_crc.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "mbedtls/base64.h"
#include "nvs.h"

#define NVS_NAMESPACE "dualeye"
#define SECTOR 4096
#define HEADER_SIZE 16
#define FRAME_ENTRY_SIZE 12
#define MAX_FRAMES 512
/* Base64 of one `media/write`, decoded: what fits a link frame with room to spare. */
#define MAX_CHUNK 2304

static const char *TAG = "media";

static const char *const SLOT_KEYS[BOARD_LCD_COUNT] = {[UI_SCREEN_CPU] = "img_left", [UI_SCREEN_GPU] = "img_right"};
static const char *const SCREEN_NAMES[BOARD_LCD_COUNT] = {[UI_SCREEN_CPU] = "left", [UI_SCREEN_GPU] = "right"};

typedef struct {
    /* The slot mapped into the address space, while it holds an image. */
    const uint8_t *data;
    esp_partition_mmap_handle_t map;
    uint32_t size;
    int frames;
} slot_t;

/* The upload in progress. */
typedef struct {
    bool active;
    int screen;
    uint32_t size;
    uint32_t crc;
    uint32_t want_crc;
    uint32_t written;
    /* Everything below this offset is erased. */
    uint32_t erased;
} upload_t;

static const esp_partition_t *s_part;
static uint32_t s_slot_size;
static slot_t s_slots[BOARD_LCD_COUNT];
static upload_t s_upload;
static uint32_t s_generation;
/* Held while a slot is mapped or unmapped, and while it's decoded. */
static SemaphoreHandle_t s_lock;

static uint16_t rd16(const uint8_t *p)
{
    return (uint16_t) (p[0] | (p[1] << 8));
}

static uint32_t rd32(const uint8_t *p)
{
    return (uint32_t) p[0] | ((uint32_t) p[1] << 8) | ((uint32_t) p[2] << 16) | ((uint32_t) p[3] << 24);
}

/* Frames in a well-formed image of `size` bytes at `p`; 0 if it isn't one. */
static int check_image(const uint8_t *p, uint32_t size)
{
    if (size < HEADER_SIZE || memcmp(p, "DEIM", 4) != 0 || p[4] != 1) {
        return 0;
    }
    int frames = rd16(p + 6);
    if (frames < 1 || frames > MAX_FRAMES || rd16(p + 8) != MEDIA_WIDTH || rd16(p + 10) != MEDIA_HEIGHT) {
        return 0;
    }
    if (HEADER_SIZE + (uint32_t) frames * FRAME_ENTRY_SIZE > size) {
        return 0;
    }
    for (int i = 0; i < frames; i++) {
        const uint8_t *e = p + HEADER_SIZE + i * FRAME_ENTRY_SIZE;
        uint32_t off = rd32(e), len = rd32(e + 4);
        if (off > size || len > size - off || e[10] > MEDIA_CODING_RLE) {
            return 0;
        }
        if (e[10] == MEDIA_CODING_RAW && len != MEDIA_WIDTH * MEDIA_HEIGHT * 2) {
            return 0;
        }
    }
    return frames;
}

static void unmap(int screen)
{
    slot_t *s = &s_slots[screen];
    if (s->data != NULL) {
        esp_partition_munmap(s->map);
    }
    memset(s, 0, sizeof(*s));
}

static bool map(int screen, uint32_t size)
{
    slot_t *s = &s_slots[screen];
    const void *ptr = NULL;
    esp_err_t err = esp_partition_mmap(s_part, (size_t) screen * s_slot_size, size, ESP_PARTITION_MMAP_DATA, &ptr, &s->map);
    if (err != ESP_OK) {
        ESP_LOGW(TAG, "mapping the %s image failed: %s", SCREEN_NAMES[screen], esp_err_to_name(err));
        return false;
    }
    int frames = check_image(ptr, size);
    if (frames == 0) {
        esp_partition_munmap(s->map);
        ESP_LOGW(TAG, "the %s image is damaged", SCREEN_NAMES[screen]);
        return false;
    }
    s->data = ptr;
    s->size = size;
    s->frames = frames;
    return true;
}

static void save_size(int screen, uint32_t size)
{
    nvs_handle_t nvs;
    if (nvs_open(NVS_NAMESPACE, NVS_READWRITE, &nvs) != ESP_OK) {
        return;
    }
    if (nvs_set_u32(nvs, SLOT_KEYS[screen], size) == ESP_OK) {
        nvs_commit(nvs);
    }
    nvs_close(nvs);
}

void media_init(void)
{
    s_lock = xSemaphoreCreateMutex();
    s_part = esp_partition_find_first(ESP_PARTITION_TYPE_DATA, ESP_PARTITION_SUBTYPE_ANY, "media");
    if (s_part == NULL) {
        ESP_LOGW(TAG, "no media partition: no images");
        return;
    }
    s_slot_size = (s_part->size / BOARD_LCD_COUNT) & ~(SECTOR - 1);
    nvs_handle_t nvs;
    if (nvs_open(NVS_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return;
    }
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        uint32_t size = 0;
        if (nvs_get_u32(nvs, SLOT_KEYS[i], &size) == ESP_OK && size > 0 && size <= s_slot_size && map(i, size)) {
            ESP_LOGI(TAG, "%s image: %d frames, %u bytes", SCREEN_NAMES[i], s_slots[i].frames, (unsigned) size);
        }
    }
    nvs_close(nvs);
}

bool media_present(int screen)
{
    return screen >= 0 && screen < BOARD_LCD_COUNT && s_slots[screen].data != NULL;
}

int media_frame_count(int screen)
{
    return media_present(screen) ? s_slots[screen].frames : 0;
}

uint32_t media_generation(void)
{
    return s_generation;
}

static bool decode_rle(const uint8_t *in, uint32_t len, uint16_t *out)
{
    const uint32_t total = MEDIA_WIDTH * MEDIA_HEIGHT;
    uint32_t px = 0, i = 0;
    while (i + 2 <= len && px < total) {
        uint16_t word = rd16(in + i);
        i += 2;
        uint32_t n = (word & 0x7FFF) + 1u;
        if (n > total - px) {
            return false;
        }
        if (word & 0x8000) {
            if (i + 2 > len) {
                return false;
            }
            uint16_t color = rd16(in + i);
            i += 2;
            for (uint32_t k = 0; k < n; k++) {
                out[px++] = color;
            }
        } else {
            if (i + 2 * n > len) {
                return false;
            }
            memcpy(out + px, in + i, 2 * n);
            px += n;
            i += 2 * n;
        }
    }
    return px == total;
}

bool media_decode(int screen, int index, uint16_t *out, uint16_t *delay_ms)
{
    if (s_lock == NULL || xSemaphoreTake(s_lock, 0) != pdTRUE) {
        // An upload is changing the slots: try again on the next frame.
        return false;
    }
    bool ok = false;
    const slot_t *s = &s_slots[screen];
    if (screen >= 0 && screen < BOARD_LCD_COUNT && s->data != NULL && index >= 0 && index < s->frames) {
        const uint8_t *e = s->data + HEADER_SIZE + index * FRAME_ENTRY_SIZE;
        const uint8_t *frame = s->data + rd32(e);
        uint32_t len = rd32(e + 4);
        *delay_ms = rd16(e + 8);
        if (e[10] == MEDIA_CODING_RAW) {
            memcpy(out, frame, len);
            ok = true;
        } else {
            ok = decode_rle(frame, len, out);
        }
    }
    xSemaphoreGive(s_lock);
    return ok;
}

static bool screen_param(const cJSON *params, int *screen)
{
    const cJSON *name = cJSON_GetObjectItemCaseSensitive(params, "screen");
    for (int i = 0; i < BOARD_LCD_COUNT; i++) {
        if (cJSON_IsString(name) && strcmp(name->valuestring, SCREEN_NAMES[i]) == 0) {
            *screen = i;
            return true;
        }
    }
    return false;
}

/* Forget `screen`'s image: off the screen, out of NVS. */
static void drop(int screen)
{
    xSemaphoreTake(s_lock, portMAX_DELAY);
    unmap(screen);
    s_generation++;
    xSemaphoreGive(s_lock);
    save_size(screen, 0);
}

static cJSON *begin(const cJSON *params, const char **message)
{
    int screen = 0;
    const cJSON *size = cJSON_GetObjectItemCaseSensitive(params, "size");
    const cJSON *crc = cJSON_GetObjectItemCaseSensitive(params, "crc32");
    if (!screen_param(params, &screen) || !cJSON_IsNumber(size) || !cJSON_IsNumber(crc)) {
        *message = "expected {\"screen\": \"left\" | \"right\", \"size\": bytes, \"crc32\": number}";
        return NULL;
    }
    if (size->valuedouble < HEADER_SIZE || size->valuedouble > s_slot_size) {
        static char why[64];
        snprintf(why, sizeof(why), "size must be up to %u bytes", (unsigned) s_slot_size);
        *message = why;
        return NULL;
    }
    drop(screen);
    s_upload = (upload_t) {
        .active = true,
        .screen = screen,
        .size = (uint32_t) size->valuedouble,
        .want_crc = (uint32_t) crc->valuedouble,
    };
    cJSON *result = cJSON_CreateObject();
    cJSON_AddNumberToObject(result, "chunk", MAX_CHUNK);
    return result;
}

static cJSON *write_chunk(const cJSON *params, const char **message)
{
    static uint8_t buf[MAX_CHUNK];
    const cJSON *offset = cJSON_GetObjectItemCaseSensitive(params, "offset");
    const cJSON *data = cJSON_GetObjectItemCaseSensitive(params, "data");
    if (!s_upload.active) {
        *message = "no upload: send media/begin first";
        return NULL;
    }
    if (!cJSON_IsNumber(offset) || !cJSON_IsString(data)) {
        *message = "expected {\"offset\": bytes, \"data\": base64}";
        return NULL;
    }
    if ((uint32_t) offset->valuedouble != s_upload.written) {
        *message = "chunks must come in order";
        return NULL;
    }
    size_t len = 0;
    const char *b64 = data->valuestring;
    if (mbedtls_base64_decode(buf, sizeof(buf), &len, (const unsigned char *) b64, strlen(b64)) != 0) {
        *message = "data isn't base64, or is over the chunk size";
        return NULL;
    }
    if (len > s_upload.size - s_upload.written) {
        *message = "more data than the size given";
        return NULL;
    }
    uint32_t base = (uint32_t) s_upload.screen * s_slot_size;
    uint32_t end = s_upload.written + len;
    if (end > s_upload.erased) {
        uint32_t upto = (end + SECTOR - 1) & ~(SECTOR - 1);
        esp_err_t err = esp_partition_erase_range(s_part, base + s_upload.erased, upto - s_upload.erased);
        if (err != ESP_OK) {
            s_upload.active = false;
            *message = "erasing the flash failed";
            return NULL;
        }
        s_upload.erased = upto;
    }
    if (esp_partition_write(s_part, base + s_upload.written, buf, len) != ESP_OK) {
        s_upload.active = false;
        *message = "writing the flash failed";
        return NULL;
    }
    s_upload.crc = esp_rom_crc32_le(s_upload.crc, buf, len);
    s_upload.written = end;
    cJSON *result = cJSON_CreateObject();
    cJSON_AddNumberToObject(result, "written", s_upload.written);
    return result;
}

static cJSON *end(const char **message)
{
    if (!s_upload.active) {
        *message = "no upload: send media/begin first";
        return NULL;
    }
    s_upload.active = false;
    if (s_upload.written != s_upload.size) {
        *message = "the upload ended short of its size";
        return NULL;
    }
    if (s_upload.crc != s_upload.want_crc) {
        *message = "CRC-32 mismatch: the image didn't arrive intact";
        return NULL;
    }
    int screen = s_upload.screen;
    xSemaphoreTake(s_lock, portMAX_DELAY);
    bool ok = map(screen, s_upload.size);
    s_generation++;
    xSemaphoreGive(s_lock);
    if (!ok) {
        *message = "not an image this firmware can show";
        return NULL;
    }
    save_size(screen, s_upload.size);
    ESP_LOGI(TAG, "%s image: %d frames, %u bytes", SCREEN_NAMES[screen], s_slots[screen].frames, (unsigned) s_upload.size);
    cJSON *result = cJSON_CreateObject();
    cJSON_AddNumberToObject(result, "frames", s_slots[screen].frames);
    return result;
}

cJSON *media_rpc(const char *method, const cJSON *params, const char **message)
{
    if (s_part == NULL) {
        *message = "this board has no media partition: flash the firmware from the app again";
        return NULL;
    }
    if (strcmp(method, "media/begin") == 0) {
        return begin(params, message);
    }
    if (strcmp(method, "media/write") == 0) {
        return write_chunk(params, message);
    }
    if (strcmp(method, "media/end") == 0) {
        return end(message);
    }
    if (strcmp(method, "media/clear") == 0) {
        int screen = 0;
        if (!screen_param(params, &screen)) {
            *message = "expected {\"screen\": \"left\" | \"right\"}";
            return NULL;
        }
        if (s_upload.active && s_upload.screen == screen) {
            s_upload.active = false;
        }
        drop(screen);
        return cJSON_CreateObject();
    }
    if (strcmp(method, "media/info") == 0) {
        cJSON *result = cJSON_CreateObject();
        cJSON_AddNumberToObject(result, "slot_size", s_slot_size);
        cJSON_AddNumberToObject(result, "chunk", MAX_CHUNK);
        cJSON *screens = cJSON_AddObjectToObject(result, "screens");
        for (int i = 0; i < BOARD_LCD_COUNT; i++) {
            cJSON *s = cJSON_AddObjectToObject(screens, SCREEN_NAMES[i]);
            cJSON_AddNumberToObject(s, "frames", s_slots[i].frames);
            cJSON_AddNumberToObject(s, "bytes", s_slots[i].size);
        }
        return result;
    }
    *message = "method not found";
    return NULL;
}
