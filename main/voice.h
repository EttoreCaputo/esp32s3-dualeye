#pragma once

/* Always-on wake word: I2S mic + speaker loopback -> ESP-SR AFE (AEC, NS,
 * AGC) -> WakeNet, on core 1. Drives the voice state the "eyes" overlay
 * shows and tells the host with `wake` and `voice_state` notifications
 * (docs/protocol.md). */

#include <stdbool.h>

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
 * after board_audio_init(). Failing leaves the rest of the firmware running
 * without voice. */
void voice_start(bool muted);

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

/** The WakeNet model in use, e.g. "wn9_hiesp", or NULL without voice. */
const char *voice_wake_model(void);
/** Its wake word as people say it, e.g. "Hi ESP". */
const char *voice_wake_word(void);

#ifdef __cplusplus
}
#endif
