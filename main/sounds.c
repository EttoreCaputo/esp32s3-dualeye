#include "sounds.h"

#include <math.h>
#include <string.h>

#include "board_audio.h"

/* Every note fades in and out over this much (less on a short one), so it
 * doesn't click; at this amplitude (the speaker volume applies on top). */
#define FADE_MS 8
#define AMPLITUDE 9000.0f
/* A pluck has died down to e^-PLUCK_DECAY by the end of its note. */
#define PLUCK_DECAY 5.0f

#define PI2 6.2831853f

typedef enum {
    W_SINE,
    /* Softer than a square, brighter than a sine: a whistle, a blip. */
    W_TRI,
    /* The first three harmonics: a voice-like hum. */
    W_BUZZ,
    /* White noise through a low-pass at the note's pitch: a puff, a sneeze. */
    W_NOISE,
    /* The buzz with breath on it: a yawn, a sigh. */
    W_BREATH,
} wave_t;

/* How the loudness goes over a note, on top of the fades. */
#define E_PLUCK 0x1 /* struck, then dying away */
#define E_SWELL 0x2 /* rising and falling, half a sine */
#define E_TREM 0x4  /* pulsing at `rate`: a purr, a snore's rattle */

typedef struct {
    /* Hz at the start and at the end, gliding evenly in pitch; 0 for a rest. */
    uint16_t f0, f1;
    uint16_t ms;
    uint8_t wave;
    /* Percent of AMPLITUDE. */
    uint8_t vol;
    /* Vibrato, in percent of the pitch, at `rate` Hz (the tremolo's too). */
    uint8_t vib;
    uint8_t rate;
    uint8_t env;
} tone_t;

#define REST(ms) {0, 0, ms, 0, 0, 0, 0, 0}

/* The earcons, as they always were. G5 C6, short: over well within the wake
 * word's tail, which the VAD ignores. */
static const tone_t WAKE[] = {{784, 784, 60, W_SINE, 100, 0, 0, 0}, REST(20), {1047, 1047, 90, W_SINE, 100, 0, 0, 0}};
static const tone_t ERROR[] = {{587, 587, 110, W_SINE, 100, 0, 0, 0}, REST(40), {392, 392, 200, W_SINE, 100, 0, 0, 0}};
/* C6 E6 G6, once: short, so the wake word has the silence between to be heard in. */
static const tone_t ALARM[] = {
    {1047, 1047, 110, W_SINE, 100, 0, 0, 0}, REST(30), {1319, 1319, 110, W_SINE, 100, 0, 0, 0}, REST(30), {1568, 1568, 220, W_SINE, 100, 0, 0, 0},
};
/* A tick at the new volume. */
static const tone_t VOLUME[] = {{880, 880, 120, W_SINE, 100, 0, 0, E_PLUCK}};

/* Powering up: a rising hum, then "hi!". */
static const tone_t BOOT[] = {
    {180, 700, 320, W_TRI, 55, 0, 0, E_SWELL}, REST(60),
    {900, 1300, 80, W_SINE, 70, 0, 0, 0}, {1300, 1100, 110, W_SINE, 60, 2, 10, 0},
};
/* "Hi-iii!": the host is back. */
static const tone_t HELLO[] = {{700, 1100, 90, W_SINE, 80, 0, 0, 0}, REST(30), {900, 1500, 170, W_SINE, 80, 3, 12, 0}};
static const tone_t MUTE[] = {{660, 330, 170, W_SINE, 80, 0, 0, E_SWELL}};
static const tone_t UNMUTE[] = {{330, 660, 170, W_SINE, 80, 0, 0, E_SWELL}};
/* "Mm-hm": got it, thinking. */
static const tone_t THINK[] = {{500, 600, 110, W_TRI, 45, 0, 0, 0}, REST(40), {600, 520, 130, W_TRI, 40, 0, 0, E_SWELL}};
static const tone_t CHIRP[] = {{1500, 2600, 50, W_SINE, 70, 0, 0, 0}, REST(25), {1800, 3000, 65, W_SINE, 60, 0, 0, 0}};
/* A happy trill, rising. */
static const tone_t HAPPY[] = {
    {900, 1200, 70, W_TRI, 65, 0, 0, 0}, {1200, 1500, 70, W_TRI, 65, 0, 0, 0}, {1500, 1900, 160, W_SINE, 70, 4, 14, 0},
};
/* "Hee-hee-hee-hee", stepping down. */
static const tone_t GIGGLE[] = {
    {1400, 1650, 55, W_BUZZ, 60, 0, 0, E_SWELL}, REST(40), {1300, 1550, 55, W_BUZZ, 60, 0, 0, E_SWELL}, REST(40),
    {1200, 1450, 55, W_BUZZ, 60, 0, 0, E_SWELL}, REST(40), {1100, 1350, 80, W_BUZZ, 60, 0, 0, E_SWELL},
};
/* Faster and faster, higher and higher. */
static const tone_t EXCITED[] = {
    {900, 1400, 55, W_SINE, 70, 0, 0, 0}, REST(20), {1000, 1600, 55, W_SINE, 70, 0, 0, 0}, REST(20),
    {1200, 1900, 55, W_SINE, 70, 0, 0, 0}, REST(20), {1400, 2300, 110, W_SINE, 75, 5, 16, 0},
};
/* A big "aaa-ooh". */
static const tone_t YAWN[] = {
    {300, 540, 380, W_BREATH, 45, 0, 0, E_SWELL},
    {540, 230, 760, W_BREATH, 50, 2, 5, E_SWELL},
};
/* "Ah... ah... CHOO!" */
static const tone_t SNEEZE[] = {
    {500, 700, 220, W_BREATH, 40, 0, 0, E_SWELL}, REST(120),
    {560, 820, 260, W_BREATH, 45, 0, 0, E_SWELL}, REST(80),
    {6000, 1400, 200, W_NOISE, 100, 0, 0, E_PLUCK},
};
/* Breathing in with a rattle, out with a whoosh. */
static const tone_t SNORE[] = {
    {380, 620, 900, W_NOISE, 80, 0, 30, E_SWELL | E_TREM}, REST(150),
    {900, 350, 750, W_NOISE, 40, 0, 0, E_SWELL},
};
static const tone_t PURR[] = {
    {170, 180, 700, W_BUZZ, 50, 0, 24, E_SWELL | E_TREM}, REST(80),
    {180, 165, 800, W_BUZZ, 45, 0, 22, E_SWELL | E_TREM},
};
static const tone_t SIGH[] = {
    {2600, 900, 300, W_NOISE, 45, 0, 0, E_SWELL},
    {420, 250, 800, W_BREATH, 40, 0, 0, E_SWELL},
};
/* A sharp breath in, "whoop!". */
static const tone_t GASP[] = {{3000, 4500, 90, W_NOISE, 55, 0, 0, E_SWELL}, {700, 1500, 170, W_SINE, 80, 3, 16, 0}};
/* "Huh?!", woken up. */
static const tone_t STARTLE[] = {{380, 1000, 200, W_BUZZ, 55, 0, 0, E_SWELL}};
/* A spring let go. */
static const tone_t BOING[] = {
    {200, 620, 450, W_SINE, 80, 18, 9, E_PLUCK},
    {520, 300, 900, W_TRI, 45, 10, 6, E_SWELL},
};
/* "Oh-nooo". */
static const tone_t AWW[] = {
    {700, 450, 550, W_TRI, 60, 3, 6, E_SWELL},
    {450, 340, 500, W_TRI, 45, 3, 6, E_SWELL},
};
/* A low, fuming rumble, then "hmph!". */
static const tone_t GRUMBLE[] = {
    {150, 120, 700, W_BUZZ, 60, 0, 16, E_TREM}, REST(60),
    {260, 180, 180, W_BREATH, 60, 0, 0, E_PLUCK},
};
/* Lub-dub; once a second makes a heart. */
static const tone_t HEARTBEAT[] = {
    {240, 140, 90, W_BUZZ, 100, 0, 0, E_PLUCK}, REST(140), {220, 130, 110, W_BUZZ, 85, 0, 0, E_PLUCK},
};
/* "Mwah!", then a dreamy "aww". */
static const tone_t SMOOCH[] = {
    {5000, 5000, 30, W_NOISE, 90, 0, 0, E_PLUCK}, REST(80),
    {700, 1100, 260, W_SINE, 60, 3, 8, E_SWELL}, {1100, 900, 380, W_SINE, 55, 4, 7, E_SWELL},
};
/* "Hm?" */
static const tone_t HMM[] = {{480, 820, 280, W_BUZZ, 40, 2, 6, E_SWELL}};
/* "Huh? Hmm." */
static const tone_t CONFUSED[] = {
    {400, 720, 190, W_BUZZ, 45, 0, 0, E_SWELL}, REST(70), {620, 430, 240, W_BUZZ, 40, 0, 0, E_SWELL},
};
static const tone_t UH_OH[] = {{700, 700, 140, W_TRI, 70, 0, 0, 0}, REST(40), {520, 490, 260, W_TRI, 70, 0, 0, E_SWELL}};
static const tone_t TADA[] = {
    {784, 784, 90, W_TRI, 60, 0, 0, E_PLUCK}, {988, 988, 90, W_TRI, 60, 0, 0, E_PLUCK},
    {1175, 1175, 90, W_TRI, 60, 0, 0, E_PLUCK}, {1568, 1568, 380, W_SINE, 80, 2, 6, E_PLUCK},
};
/* "Uh-huh!" */
static const tone_t YES[] = {{500, 700, 100, W_TRI, 60, 0, 0, 0}, REST(40), {700, 1000, 150, W_TRI, 60, 0, 0, E_SWELL}};
/* "Uh-uh." */
static const tone_t NOPE[] = {{300, 280, 130, W_BUZZ, 50, 0, 0, 0}, REST(70), {300, 240, 170, W_BUZZ, 50, 0, 0, E_SWELL}};
/* A little tune: G E C' A G. */
static const tone_t SING[] = {
    {784, 784, 190, W_TRI, 55, 0, 0, E_SWELL}, REST(30), {659, 659, 190, W_TRI, 55, 0, 0, E_SWELL}, REST(30),
    {1047, 1047, 190, W_TRI, 55, 0, 0, E_SWELL}, REST(30), {880, 880, 190, W_TRI, 55, 0, 0, E_SWELL}, REST(30),
    {784, 784, 420, W_TRI, 55, 3, 6, E_SWELL},
};
/* Two bars of kick and hi-hat at 120 bpm. */
#define KICK {260, 80, 100, W_BUZZ, 100, 0, 0, E_PLUCK}
#define HAT {7000, 7000, 40, W_NOISE, 60, 0, 0, E_PLUCK}
static const tone_t BEAT[] = {
    KICK, REST(150), HAT, REST(210), KICK, REST(150), HAT, REST(210),
    KICK, REST(150), HAT, REST(210), KICK, REST(150), HAT, REST(210),
};
/* Robot beeps over a sweep. */
static const tone_t SCAN[] = {
    {1600, 1600, 40, W_TRI, 50, 0, 0, 0}, REST(80), {2000, 2000, 40, W_TRI, 50, 0, 0, 0}, REST(80),
    {1600, 1600, 40, W_TRI, 50, 0, 0, 0}, REST(80), {800, 2400, 600, W_SINE, 35, 0, 0, E_SWELL},
};
/* Target locked. */
static const tone_t LOCK[] = {
    {1200, 1200, 60, W_TRI, 55, 0, 0, 0}, REST(50), {1200, 1200, 60, W_TRI, 55, 0, 0, 0}, REST(50), {1800, 1800, 260, W_SINE, 60, 0, 0, 0},
};
static const tone_t HICCUP[] = {{650, 1300, 80, W_BUZZ, 80, 0, 0, E_SWELL}};
/* Wolf whistle. */
static const tone_t WHISTLE[] = {
    {900, 2200, 240, W_SINE, 60, 0, 0, 0}, REST(90), {900, 2400, 160, W_SINE, 60, 0, 0, 0}, {2400, 800, 380, W_SINE, 55, 0, 0, E_SWELL},
};
/* A click and a ding. */
static const tone_t WINK[] = {{3500, 3500, 25, W_NOISE, 90, 0, 0, E_PLUCK}, REST(30), {1300, 1750, 120, W_SINE, 70, 0, 0, E_SWELL}};
static const tone_t BOOP[] = {{600, 600, 80, W_SINE, 75, 0, 0, E_SWELL}, REST(40), {900, 900, 100, W_SINE, 75, 0, 0, E_SWELL}};
/* Air out through the lips: "pfff". */
static const tone_t PFFT[] = {{2800, 1100, 420, W_NOISE, 50, 0, 18, E_SWELL}};
static const tone_t EEP[] = {{1600, 2100, 100, W_SINE, 60, 0, 0, E_SWELL}};
/* Flap-flap-flap-flap. */
#define FLAP {3200, 2000, 40, W_NOISE, 90, 0, 0, E_SWELL}
static const tone_t FLUTTER[] = {FLAP, REST(70), FLAP, REST(70), FLAP, REST(70), FLAP};
/* "Heh heh heh", low and sly. */
static const tone_t MISCHIEF[] = {
    {330, 300, 90, W_BREATH, 70, 0, 0, E_SWELL}, REST(70), {300, 270, 90, W_BREATH, 70, 0, 0, E_SWELL}, REST(70),
    {270, 240, 130, W_BREATH, 70, 0, 0, E_SWELL},
};
/* A puppy's whine. */
static const tone_t WHIMPER[] = {
    {900, 1150, 260, W_SINE, 45, 5, 7, E_SWELL}, REST(80), {1050, 780, 420, W_SINE, 40, 5, 6, E_SWELL},
};
static const tone_t BOO[] = {{300, 620, 220, W_BUZZ, 60, 0, 0, E_SWELL}, {620, 500, 160, W_BUZZ, 45, 0, 0, E_SWELL}};
/* Bits going wrong. */
static const tone_t GLITCH[] = {
    {2400, 2400, 30, W_TRI, 50, 0, 0, 0}, {5000, 5000, 40, W_NOISE, 50, 0, 0, 0}, {300, 300, 30, W_TRI, 50, 0, 0, 0}, REST(50),
    {1800, 600, 60, W_TRI, 50, 0, 0, 0}, {6000, 6000, 30, W_NOISE, 50, 0, 0, 0}, REST(40), {900, 2700, 50, W_TRI, 50, 0, 0, 0},
    {4000, 4000, 30, W_NOISE, 45, 0, 0, 0}, {200, 200, 60, W_TRI, 50, 0, 0, 0},
};

typedef struct {
    const char *name;
    const tone_t *tones;
    uint8_t count;
} sound_def_t;

#define DEF(id, name, tones) [id] = {name, tones, sizeof(tones) / sizeof(tones[0])}

static const sound_def_t SOUNDS[SOUND_COUNT] = {
    DEF(SOUND_WAKE, "wake", WAKE),
    DEF(SOUND_ERROR, "error", ERROR),
    DEF(SOUND_ALARM, "alarm", ALARM),
    DEF(SOUND_VOLUME, "volume", VOLUME),
    DEF(SOUND_BOOT, "boot", BOOT),
    DEF(SOUND_HELLO, "hello", HELLO),
    DEF(SOUND_MUTE, "mute", MUTE),
    DEF(SOUND_UNMUTE, "unmute", UNMUTE),
    DEF(SOUND_THINK, "think", THINK),
    DEF(SOUND_CHIRP, "chirp", CHIRP),
    DEF(SOUND_HAPPY, "happy", HAPPY),
    DEF(SOUND_GIGGLE, "giggle", GIGGLE),
    DEF(SOUND_EXCITED, "excited", EXCITED),
    DEF(SOUND_YAWN, "yawn", YAWN),
    DEF(SOUND_SNEEZE, "sneeze", SNEEZE),
    DEF(SOUND_SNORE, "snore", SNORE),
    DEF(SOUND_PURR, "purr", PURR),
    DEF(SOUND_SIGH, "sigh", SIGH),
    DEF(SOUND_GASP, "gasp", GASP),
    DEF(SOUND_STARTLE, "startle", STARTLE),
    DEF(SOUND_BOING, "boing", BOING),
    DEF(SOUND_AWW, "aww", AWW),
    DEF(SOUND_GRUMBLE, "grumble", GRUMBLE),
    DEF(SOUND_HEARTBEAT, "heartbeat", HEARTBEAT),
    DEF(SOUND_SMOOCH, "smooch", SMOOCH),
    DEF(SOUND_HMM, "hmm", HMM),
    DEF(SOUND_CONFUSED, "confused", CONFUSED),
    DEF(SOUND_UH_OH, "uh_oh", UH_OH),
    DEF(SOUND_TADA, "tada", TADA),
    DEF(SOUND_YES, "yes", YES),
    DEF(SOUND_NOPE, "nope", NOPE),
    DEF(SOUND_SING, "sing", SING),
    DEF(SOUND_BEAT, "beat", BEAT),
    DEF(SOUND_SCAN, "scan", SCAN),
    DEF(SOUND_LOCK, "lock", LOCK),
    DEF(SOUND_HICCUP, "hiccup", HICCUP),
    DEF(SOUND_WHISTLE, "whistle", WHISTLE),
    DEF(SOUND_WINK, "wink", WINK),
    DEF(SOUND_BOOP, "boop", BOOP),
    DEF(SOUND_PFFT, "pfft", PFFT),
    DEF(SOUND_EEP, "eep", EEP),
    DEF(SOUND_FLUTTER, "flutter", FLUTTER),
    DEF(SOUND_MISCHIEF, "mischief", MISCHIEF),
    DEF(SOUND_WHIMPER, "whimper", WHIMPER),
    DEF(SOUND_BOO, "boo", BOO),
    DEF(SOUND_GLITCH, "glitch", GLITCH),
};

const char *sound_name(sound_t sound)
{
    return sound < SOUND_COUNT ? SOUNDS[sound].name : NULL;
}

sound_t sound_find(const char *name)
{
    for (int i = 0; i < SOUND_COUNT; i++) {
        if (strcmp(SOUNDS[i].name, name) == 0) {
            return (sound_t) i;
        }
    }
    return SOUND_COUNT;
}

static void tone_begin(synth_t *s)
{
    const tone_t *t = &((const tone_t *) s->tones)[s->tone];
    s->k = 0;
    s->n = (uint32_t) t->ms * (BOARD_AUDIO_SAMPLE_RATE / 1000);
    s->freq = t->f0;
    s->step = t->f0 > 0 && t->f1 > 0 && s->n > 0 ? powf((float) t->f1 / t->f0, 1.0f / s->n) : 1.0f;
    s->decay = 1.0f;
    s->decay_step = s->n > 0 ? expf(-PLUCK_DECAY / s->n) : 1.0f;
}

void synth_begin(synth_t *s, sound_t sound)
{
    memset(s, 0, sizeof(*s));
    s->rng = 0x9E3779B9u;
    if (sound >= SOUND_COUNT) {
        return;
    }
    s->tones = SOUNDS[sound].tones;
    s->count = SOUNDS[sound].count;
    tone_begin(s);
}

/** White noise, -1..1. */
static float noise(synth_t *s)
{
    s->rng ^= s->rng << 13;
    s->rng ^= s->rng >> 17;
    s->rng ^= s->rng << 5;
    return (float) (int32_t) s->rng / 2147483648.0f;
}

/** The noise through two one-pole low-passes at `corner`, made up for
 * what they take away, so that a low one isn't much quieter. */
static float filtered_noise(synth_t *s, float corner)
{
    float a = 1.0f - expf(-PI2 * corner / BOARD_AUDIO_SAMPLE_RATE);
    s->lp1 += a * (noise(s) - s->lp1);
    s->lp2 += a * (s->lp1 - s->lp2);
    return s->lp2 * 0.9f / sqrtf(a / (2.0f - a));
}

uint32_t synth_render(synth_t *s, int16_t *out, uint32_t max)
{
    const tone_t *tones = s->tones;
    uint32_t done = 0;
    while (done < max && s->tone < s->count) {
        const tone_t *t = &tones[s->tone];
        if (s->k >= s->n) {
            if (++s->tone < s->count) {
                tone_begin(s);
            }
            continue;
        }
        const float rate = (float) BOARD_AUDIO_SAMPLE_RATE;
        uint32_t fade = FADE_MS * (BOARD_AUDIO_SAMPLE_RATE / 1000);
        if (fade > s->n / 4) {
            fade = s->n / 4 > 0 ? s->n / 4 : 1;
        }
        for (; done < max && s->k < s->n; done++, s->k++) {
            uint32_t k = s->k;
            if (t->f0 == 0) {
                out[done] = 0;
                continue;
            }
            float g = AMPLITUDE * t->vol / 100.0f;
            if (k < fade) {
                g *= (float) k / fade;
            } else if (s->n - k < fade) {
                g *= (float) (s->n - k) / fade;
            }
            float secs = (float) k / rate;
            if (t->env & E_SWELL) {
                g *= sinf(3.1415927f * k / s->n);
            }
            if (t->env & E_PLUCK) {
                g *= s->decay;
                s->decay *= s->decay_step;
            }
            if (t->env & E_TREM) {
                g *= 0.5f - 0.5f * cosf(PI2 * t->rate * secs);
            }
            float f = s->freq;
            s->freq *= s->step;
            if (t->vib > 0) {
                f *= 1.0f + t->vib / 100.0f * sinf(PI2 * t->rate * secs);
            }
            float v;
            if (t->wave == W_NOISE) {
                v = filtered_noise(s, f);
            } else {
                s->phase += PI2 * f / rate;
                if (s->phase >= PI2) {
                    s->phase -= PI2;
                }
                float p = s->phase;
                switch (t->wave) {
                case W_TRI:
                    v = 4.0f * fabsf(p / PI2 - 0.5f) - 1.0f;
                    break;
                case W_BUZZ:
                    v = (sinf(p) + 0.5f * sinf(2 * p) + 0.25f * sinf(3 * p)) / 1.4f;
                    break;
                case W_BREATH:
                    v = 0.7f * (sinf(p) + 0.5f * sinf(2 * p) + 0.25f * sinf(3 * p)) / 1.4f
                        + 0.5f * filtered_noise(s, 4.0f * f);
                    break;
                default:
                    v = sinf(p);
                    break;
                }
            }
            float x = g * v;
            out[done] = (int16_t) (x > 32767.0f ? 32767.0f : x < -32768.0f ? -32768.0f : x);
        }
    }
    return done;
}
