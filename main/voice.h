#pragma once

/* Always-on wake word: I2S mic + speaker loopback -> ESP-SR AFE (AEC,
 * VAD) -> WakeNet, on core 1. Between conversations the pet's ears (ears.c)
 * hear the same audio. After the wake word the AFE's output is
 * streamed to the host on `audio_up` until the speaker stops (VAD), between
 * `utterance_start` and `utterance_end` notifications. Drives the voice state
 * the "eyes" overlay shows and tells the host with `wake` and `voice_state`
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
    /* Something went wrong on the host (or it didn't catch the words): a red
     * ring and the error sound, then idle. */
    VOICE_ERROR,
    VOICE_STATE_COUNT,
} voice_state_t;

/** Load the models from the `model` partition and start the tasks. Call
 * after board_audio_init(). `wake_word` is an id from voice_wake_words();
 * NULL, empty or unknown picks "alexa" (or the first WakeNet there is).
 * Failing leaves the rest of the firmware running without voice. */
void voice_start(bool muted, const char *wake_word);

/** True once the wake word runs (models found, tasks started). */
bool voice_available(void);

/** Chime as the board starts listening after the wake word (on by default). */
void voice_set_wake_sound(bool on);
bool voice_wake_sound(void);

/** Stop or restart listening for the wake word. Muted, the mic isn't read. */
void voice_set_muted(bool muted);
bool voice_muted(void);

/** Hand the mic to someone else (the audio self-test's `rec`) and back.
 * Blocks until the feed task has let go of it. Nests. */
void voice_pause(bool pause);

/** Stream an utterance as if the wake word had been heard (push-to-talk).
 * `follow_up`: the host asks right after its spoken reply, so what's said
 * next needs no wake word; it gives up sooner without speech. False without
 * voice or muted. */
bool voice_listen(bool follow_up);
/** End the utterance being streamed, if any. */
void voice_stop_listening(void);

/** The speaker is playing (playback.c). With barge-in
 * (CONFIG_DUALEYE_VOICE_BARGE_IN) the wake word stops it and starts a new
 * utterance, echo cancellation keeping the board from waking itself up;
 * without, the wake word is ignored meanwhile and for a moment after. */
void voice_set_speaking(bool speaking);

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
