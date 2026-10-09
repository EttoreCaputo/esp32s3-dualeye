#pragma once

/* What the pet hears in the room while nobody talks to it (pet.c): a bang
 * that makes it jump, two or more claps to call it, people talking nearby,
 * and how long the room has been quiet. voice.c feeds it the AFE's output
 * between conversations, after echo cancellation; only these few facts are
 * kept, nothing is recorded or sent. Muted, the mic isn't read and it hears
 * nothing. */

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    /* Fed lately: the wake word is listened for. */
    bool listening;
    /* The room's background level and the last chunk's, in dBFS. */
    float floor_db, level_db;
    /* Since the last sound worth noticing (or speech). */
    uint32_t quiet_s;
    /* Someone talked a good part of the last minute or two. */
    bool chatter;
} ears_state_t;

/** One chunk of what the mic hears, from the voice fetch task. `speech`: the
 * VAD hears someone. `deaf`: what's heard is the board's own (the speaker
 * plays or just did) or a conversation is on, and doesn't count. */
void ears_feed(const int16_t *pcm, int samples, bool speech, bool deaf);

/** Listen to the room: on (the default) or off. */
void ears_set_on(bool on);
bool ears_on(void);

void ears_get(ears_state_t *out);

#ifdef __cplusplus
}
#endif
