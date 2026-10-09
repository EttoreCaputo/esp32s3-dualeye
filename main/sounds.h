#pragma once

/* The board's own sounds, made up on the spot from a few lines of notes each:
 * sines, a warmer buzz and filtered noise, gliding from one pitch to another,
 * with vibrato, a tremolo (a purr, a snore) and a pluck or swell. The earcons
 * (wake, error, alarm), feedback to what the host does (volume, mute, hello)
 * and the voice of the pet: what it says during the eyes' scenes. */

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    /* The earcons: always played. */
    SOUND_WAKE,
    SOUND_ERROR,
    SOUND_ALARM,
    SOUND_VOLUME,
    /* The pet's: only while pet sounds are on (playback_set_pet_sounds). */
    SOUND_BOOT,
    SOUND_HELLO,
    SOUND_MUTE,
    SOUND_UNMUTE,
    SOUND_THINK,
    SOUND_CHIRP,
    SOUND_HAPPY,
    SOUND_GIGGLE,
    SOUND_EXCITED,
    SOUND_YAWN,
    SOUND_SNEEZE,
    SOUND_SNORE,
    SOUND_PURR,
    SOUND_SIGH,
    SOUND_GASP,
    SOUND_STARTLE,
    SOUND_BOING,
    SOUND_AWW,
    SOUND_GRUMBLE,
    SOUND_HEARTBEAT,
    SOUND_SMOOCH,
    SOUND_HMM,
    SOUND_CONFUSED,
    SOUND_UH_OH,
    SOUND_TADA,
    SOUND_YES,
    SOUND_NOPE,
    SOUND_SING,
    SOUND_BEAT,
    SOUND_SCAN,
    SOUND_LOCK,
    SOUND_HICCUP,
    SOUND_WHISTLE,
    SOUND_WINK,
    SOUND_BOOP,
    SOUND_PFFT,
    SOUND_EEP,
    SOUND_FLUTTER,
    SOUND_MISCHIEF,
    SOUND_WHIMPER,
    SOUND_BOO,
    SOUND_GLITCH,
    SOUND_COUNT,
} sound_t;

/** The first of the pet's sounds. */
#define SOUND_FIRST_PET SOUND_BOOT

/** As play_sound (board_tools.c) takes it. */
const char *sound_name(sound_t sound);

/** SOUND_COUNT when there's no such sound. */
sound_t sound_find(const char *name);

/** Where a sound being rendered is. */
typedef struct {
    const void *tones;
    uint8_t count;
    uint8_t tone;
    uint32_t k, n;
    float phase, freq, step, decay, decay_step, lp1, lp2;
    uint32_t rng;
} synth_t;

void synth_begin(synth_t *s, sound_t sound);

/** Up to `max` samples of the sound at BOARD_AUDIO_SAMPLE_RATE; 0 once it's over. */
uint32_t synth_render(synth_t *s, int16_t *out, uint32_t max);

#ifdef __cplusplus
}
#endif
