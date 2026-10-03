#include "rpc.h"

#include <stdlib.h>
#include <string.h>

#include "art.h"
#include "audio_selftest.h"
#include "board_tools.h"
#include "cJSON.h"
#include "esp_app_desc.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "link.h"
#include "media.h"
#include "playback.h"
#include "ui_eyes.h"
#include "voice.h"

#define RPC_PROTOCOL 2

#define RPC_PARSE_ERROR -32700
#define RPC_INVALID_REQUEST -32600
#define RPC_METHOD_NOT_FOUND -32601
#define RPC_INVALID_PARAMS -32602
#define RPC_INTERNAL_ERROR -32603

static const char *TAG = "rpc";

static void *psram_malloc(size_t size)
{
    return heap_caps_malloc(size, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
}

static cJSON *envelope(const cJSON *id);

static void send_message(cJSON *msg)
{
    char *text = cJSON_PrintUnformatted(msg);
    if (text == NULL) {
        cJSON_Delete(msg);
        ESP_LOGE(TAG, "out of memory for a reply");
        return;
    }
    size_t len = strlen(text);
    esp_err_t err = len <= LINK_MAX_PAYLOAD ? link_send(LINK_CHAN_CTRL, text, len) : ESP_ERR_INVALID_SIZE;
    cJSON_free(text);
    if (err != ESP_OK) {
        ESP_LOGW(TAG, "reply not sent (%u bytes): %s", (unsigned) len, esp_err_to_name(err));
    }
    // A reply too long for a frame: say so, rather than leave the host waiting.
    const cJSON *id = cJSON_GetObjectItemCaseSensitive(msg, "id");
    if (err == ESP_ERR_INVALID_SIZE && id != NULL && !cJSON_IsNull(id)) {
        cJSON *reply = envelope(id);
        cJSON *e = cJSON_AddObjectToObject(reply, "error");
        cJSON_AddNumberToObject(e, "code", RPC_INTERNAL_ERROR);
        cJSON_AddStringToObject(e, "message", "reply too long for a frame");
        send_message(reply);
    }
    cJSON_Delete(msg);
}

static cJSON *envelope(const cJSON *id)
{
    cJSON *msg = cJSON_CreateObject();
    cJSON_AddStringToObject(msg, "jsonrpc", "2.0");
    cJSON_AddItemToObject(msg, "id", id != NULL ? cJSON_Duplicate(id, true) : cJSON_CreateNull());
    return msg;
}

static void reply_result(const cJSON *id, cJSON *result)
{
    cJSON *msg = envelope(id);
    cJSON_AddItemToObject(msg, "result", result);
    send_message(msg);
}

static void reply_error(const cJSON *id, int code, const char *message)
{
    cJSON *msg = envelope(id);
    cJSON *err = cJSON_AddObjectToObject(msg, "error");
    cJSON_AddNumberToObject(err, "code", code);
    cJSON_AddStringToObject(err, "message", message);
    send_message(msg);
}

/** The `hello` result, also the params of `ready`. */
static cJSON *identity(void)
{
    const esp_app_desc_t *app = esp_app_get_description();
    cJSON *id = cJSON_CreateObject();
    cJSON_AddNumberToObject(id, "protocol", RPC_PROTOCOL);
    cJSON_AddStringToObject(id, "firmware", app->version);
    cJSON_AddStringToObject(id, "idf", app->idf_ver);
    cJSON_AddStringToObject(id, "board", "dualeye");
    cJSON_AddNumberToObject(id, "max_payload", LINK_MAX_PAYLOAD);
    cJSON *channels = cJSON_AddArrayToObject(id, "channels");
    cJSON *caps = cJSON_AddArrayToObject(id, "capabilities");
    const char *base[] = {"ctrl", "metrics", "log"};
    for (int i = 0; i < 3; i++) {
        cJSON_AddItemToArray(channels, cJSON_CreateString(base[i]));
    }
    cJSON_AddItemToArray(caps, cJSON_CreateString("tools"));
    cJSON_AddItemToArray(caps, cJSON_CreateString("media"));
    // Firmware 1.3: the music face's cover art, and eyes that follow the host's pointer.
    cJSON_AddItemToArray(caps, cJSON_CreateString("music"));
    cJSON_AddItemToArray(caps, cJSON_CreateString("gaze"));
    if (voice_available()) {
        cJSON_AddItemToArray(channels, cJSON_CreateString("audio_up"));
        cJSON_AddItemToArray(caps, cJSON_CreateString("voice"));
    }
    if (playback_available()) {
        cJSON_AddItemToArray(channels, cJSON_CreateString("audio_down"));
        cJSON_AddItemToArray(caps, cJSON_CreateString("speaker"));
    }
    return id;
}

void rpc_init(void)
{
    // Requests and replies are short-lived; keep them out of internal RAM.
    cJSON_Hooks hooks = {.malloc_fn = psram_malloc, .free_fn = free};
    cJSON_InitHooks(&hooks);
}

void rpc_notify(const char *method, cJSON *params)
{
    cJSON *msg = cJSON_CreateObject();
    cJSON_AddStringToObject(msg, "jsonrpc", "2.0");
    cJSON_AddStringToObject(msg, "method", method);
    if (params != NULL) {
        cJSON_AddItemToObject(msg, "params", params);
    }
    send_message(msg);
}

void rpc_announce(void)
{
    rpc_notify("ready", identity());
}

void rpc_handle(uint8_t *payload, size_t len)
{
    cJSON *req = cJSON_ParseWithLength((const char *) payload, len);
    if (req == NULL) {
        reply_error(NULL, RPC_PARSE_ERROR, "parse error");
        return;
    }
    const cJSON *id = cJSON_GetObjectItemCaseSensitive(req, "id");
    const cJSON *method = cJSON_GetObjectItemCaseSensitive(req, "method");
    const cJSON *params = cJSON_GetObjectItemCaseSensitive(req, "params");
    if (!cJSON_IsObject(req) || !cJSON_IsString(method)) {
        reply_error(id, RPC_INVALID_REQUEST, "invalid request");
        cJSON_Delete(req);
        return;
    }
    // Requests without an id are notifications: run, don't answer.
    const bool answer = id != NULL;
    const char *name = method->valuestring;
    cJSON *result = NULL;
    int code = 0;
    const char *message = NULL;

    if (strcmp(name, "hello") == 0) {
        result = identity();
    } else if (strcmp(name, "tools/list") == 0) {
        // Room for the envelope and nextCursor around the tools.
        result = board_tools_list(params, LINK_MAX_PAYLOAD - 128);
    } else if (strcmp(name, "tools/call") == 0) {
        const cJSON *tool = cJSON_GetObjectItemCaseSensitive(params, "name");
        const cJSON *args = cJSON_GetObjectItemCaseSensitive(params, "arguments");
        if (!cJSON_IsString(tool) || (args != NULL && !cJSON_IsObject(args))) {
            code = RPC_INVALID_PARAMS;
            message = "expected {\"name\": string, \"arguments\": object}";
        } else if ((result = board_tools_call(tool->valuestring, args)) == NULL) {
            code = RPC_INVALID_PARAMS;
            message = "unknown tool";
        }
    } else if (strcmp(name, "voice/state") == 0) {
        // The host's voice pipeline says what the eyes show (thinking, speaking, ...).
        const cJSON *state = cJSON_GetObjectItemCaseSensitive(params, "state");
        voice_state_t s;
        if (!cJSON_IsString(state) || !voice_state_from_name(state->valuestring, &s)) {
            code = RPC_INVALID_PARAMS;
            message = "expected {\"state\": \"idle\" | \"listening\" | \"thinking\" | \"speaking\" | \"error\"}";
        } else if (!voice_available()) {
            code = RPC_INVALID_PARAMS;
            message = "voice not available";
        } else {
            voice_set_state(s);
            result = cJSON_CreateObject();
        }
    } else if (strcmp(name, "voice/listen") == 0) {
        // Push-to-talk, or the follow-up after a reply: stream an utterance
        // without the wake word.
        const cJSON *follow_up = cJSON_GetObjectItemCaseSensitive(params, "follow_up");
        if (!voice_listen(cJSON_IsTrue(follow_up))) {
            code = RPC_INVALID_PARAMS;
            message = voice_available() ? "microphone muted" : "voice not available";
        } else {
            result = cJSON_CreateObject();
        }
    } else if (strcmp(name, "voice/stop") == 0) {
        voice_stop_listening();
        result = cJSON_CreateObject();
    } else if (strcmp(name, "audio/stop") == 0) {
        // Stop talking: what's buffered is dropped.
        playback_stop();
        result = cJSON_CreateObject();
    } else if (strncmp(name, "media/", 6) == 0) {
        // Images for the image face: begin, write..., end; clear; info.
        result = media_rpc(name, params, &message);
        code = RPC_INVALID_PARAMS;
        if (result == NULL && strcmp(message, "method not found") == 0) {
            code = RPC_METHOD_NOT_FOUND;
        }
    } else if (strncmp(name, "music/", 6) == 0) {
        // Cover art for the music face.
        result = art_rpc(name, params, &message);
        code = RPC_INVALID_PARAMS;
        if (result == NULL && strcmp(message, "method not found") == 0) {
            code = RPC_METHOD_NOT_FOUND;
        }
    } else if (strcmp(name, "eyes/gaze") == 0) {
        // Where the host's pointer is, -1..1 each way; sent as a notification
        // many times a second while it moves.
        const cJSON *x = cJSON_GetObjectItemCaseSensitive(params, "x");
        const cJSON *y = cJSON_GetObjectItemCaseSensitive(params, "y");
        if (!cJSON_IsNumber(x) || !cJSON_IsNumber(y)) {
            code = RPC_INVALID_PARAMS;
            message = "expected {\"x\": -1..1, \"y\": -1..1}";
        } else {
            ui_eyes_set_gaze((float) x->valuedouble, (float) y->valuedouble);
            result = cJSON_CreateObject();
        }
    } else if (strcmp(name, "debug/audio") == 0) {
        const cJSON *cmd = cJSON_GetObjectItemCaseSensitive(params, "cmd");
        if (!cJSON_IsString(cmd)) {
            code = RPC_INVALID_PARAMS;
            message = "expected {\"cmd\": string}";
        } else {
            audio_selftest_command(cmd->valuestring);
            result = cJSON_CreateObject();
        }
    } else {
        code = RPC_METHOD_NOT_FOUND;
        message = "method not found";
    }

    if (!answer) {
        cJSON_Delete(result);
    } else if (result != NULL) {
        reply_result(id, result);
    } else {
        reply_error(id, code, message);
    }
    cJSON_Delete(req);
}
