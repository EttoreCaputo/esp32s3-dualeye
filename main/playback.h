#pragma once

/* Speech from the host: PCM on `audio_down` (docs/protocol.md) goes into a
 * jitter buffer in PSRAM and out to the ES8311 from a task on core 1. The
 * host paces the stream; the board shows "speaking" while it plays, with the
 * output level on the ring, and says `playback_end` when it's done. */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "sounds.h"

#ifdef __cplusplus
extern "C" {
#endif

/** Allocate the buffer and start the task. Call after board_audio_init(). */
void playback_start(void);

/** True once it runs. */
bool playback_available(void);

/** The link's callback for `audio_down` frames. */
void playback_receive(uint8_t *payload, size_t len);

/** Drop what's buffered and end the stream being played, if any. */
void playback_stop(void);

/** The same, because the wake word was heard over it: the host's stream
 * ends with `barge_in`. */
void playback_barge_in(void);

/** Play `sound` made on the board, after those queued before it, unless the
 * host's speech is playing. It doesn't show `speaking` and isn't reported
 * to the host. Safe from any task. */
void playback_sound(sound_t sound);

/** The pet's sounds (from SOUND_FIRST_PET on: the eyes' scenes, hello...) are
 * played, or quietly dropped. On by default. */
void playback_set_pet_sounds(bool on);
bool playback_pet_sounds(void);

/** A stream is playing (or buffering to start). */
bool playback_active(void);

#ifdef __cplusplus
}
#endif
