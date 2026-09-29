#pragma once

/* Always-on wake word: I2S mic + speaker loopback -> ESP-SR AFE (AEC,
 * VAD) -> WakeNet, on core 1. Drives the voice state the "eyes" overlay
 * shows and tells the host with `wake` and `voice_state` notifications
 * (docs/protocol.md). */

#include <stdbool.h>

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    VOICE_IDLE = 0,
    VOICE_LISTENING,
    VOICE_THINKING,
    VOICE_SPEAKING,
    VOICE_STATE_COUNT,
} voice_state_t;

/** Load the models from the `model` partition and start the tasks. Call
 * after board_audio_init(). `wake_word` is an id from voice_wake_words();
 * NULL, empty or unknown picks "alexa" (or the first WakeNet there is).
 * Failing leaves the rest of the firmware running without voice. */
void voice_start(bool muted, const char *wake_word);

/** True once the wake word runs (models found, tasks started). */
bool voice_available(void);

/** Stop or restart listening for the wake word. Muted, the mic isn't read. */
void voice_set_muted(bool muted);
bool voice_muted(void);

/** Hand the mic to someone else (the audio self-test's `rec`) and back.
 * Blocks until the feed task has let go of it. Nests. */
void voice_pause(bool pause);

voice_state_t voice_state(void);
/** What the overlay shows. The board sets LISTENING on the wake word and
 * goes back to IDLE after a timeout; the host sets the others. */
void voice_set_state(voice_state_t state);

const char *voice_state_name(voice_state_t state);
bool voice_state_from_name(const char *name, voice_state_t *out);

/** The WakeNet model in use, e.g. "wn9_alexa", or NULL without voice. */
const char *voice_wake_model(void);
/** Its wake word as people say it, e.g. "Alexa". */
const char *voice_wake_word(void);
/** And as voice_set_wake_word() takes it, e.g. "alexa". */
const char *voice_wake_word_id(void);

/** The wake-word ids the `model` partition has models for ("alexa",
 * "hiesp"), at most `max`; returns how many. */
int voice_wake_words(const char **ids, int max);

/** Listen for another wake word from now on: rebuilds the AFE, which takes
 * a moment (and waits for a self-test `rec` to finish). ESP_ERR_NOT_FOUND
 * when there's no model for `id`. */
esp_err_t voice_set_wake_word(const char *id);

#ifdef __cplusplus
}
#endif
