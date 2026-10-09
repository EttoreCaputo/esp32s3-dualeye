#pragma once

/* The pet's mood: how lively (energy), cheerful (happiness) and attached to
 * you (affection) it is, each 0..1, moved by what happens and drifting back
 * over time, kept in NVS across reboots. The mood picks the eyes' idle
 * scenes and how often they come, tints the eyes face and pitches the pet's
 * sounds. It also reacts to what the host tells: a hot CPU, music, Claude
 * finishing, the battery, the time of day, you coming back to the computer. */

#include <stdbool.h>
#include <stdint.h>

#include "metrics_model.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    PET_MOOD_CONTENT,
    PET_MOOD_HAPPY,
    PET_MOOD_EXCITED,
    PET_MOOD_LOVING,
    PET_MOOD_BORED,
    PET_MOOD_GRUMPY,
    PET_MOOD_SAD,
    PET_MOOD_SLEEPY,
    PET_MOOD_HOT,
    PET_MOOD_COUNT,
} pet_mood_t;

typedef enum {
    /* Someone said the wake word. */
    PET_EVENT_TALKED,
    /* A conversation went wrong. */
    PET_EVENT_ERROR,
    /* The host played a scene or a sound (the app, Claude's alerts). */
    PET_EVENT_PLAYED,
} pet_event_t;

/* What's going on that moves the mood, as the pet sees it. */
#define PET_NOW_HOT 0x01
#define PET_NOW_MUSIC 0x02
#define PET_NOW_CLAUDE 0x04
#define PET_NOW_BATTERY_LOW 0x08
#define PET_NOW_CHARGING 0x10
#define PET_NOW_NIGHT 0x20
#define PET_NOW_AWAY 0x40
#define PET_NOW_BORED 0x80

/** One of the reactions it had, for get_pet. */
typedef struct {
    /* What it reacted to (hot, cool, music, claude_start, claude_done,
     * battery_low, charging, goodnight, greeting, errors) and the scene. */
    const char *what;
    const char *scene;
    /* Seconds ago. */
    uint32_t ago_s;
} pet_reaction_t;

#define PET_RECENT 6

typedef struct {
    float energy, happiness, affection;
    pet_mood_t mood;
    /* Nobody has touched the computer for a while: no scenes. */
    bool away;
    /* The host sends the time: -1 until it does. */
    int minute;
    /* Seconds without input on the host; -1 when it doesn't say. */
    int idle_s;
    /* Since someone last talked or played with it, or music played. */
    uint32_t lonely_s;
    /* PET_NOW_* */
    uint32_t now;
    /* Its voice's pitch (1 as written) and the idle scenes' pace (1: every
     * 30-120 s, less is more often). */
    float pitch, pace;
    /* Newest first. */
    pet_reaction_t recent[PET_RECENT];
    int recent_count;
} pet_state_t;

/** Load the mood saved in NVS. Call after board_settings_init(). */
void pet_init(void);

/** With the latest snapshot, often (it works once a second). Takes the LVGL
 * lock to play a reaction: call without it. */
void pet_update(const metrics_snapshot_t *snap);

/** Safe from any task. */
void pet_event(pet_event_t event);

/** Reactions to what happens on the computer: on (the default) or off. */
void pet_set_reactions(bool on);
bool pet_reactions(void);

void pet_get(pet_state_t *out);
const char *pet_mood_name(pet_mood_t mood);
/** The name of PET_NOW_* bit `bit`. */
const char *pet_now_name(uint32_t bit);

/** For the idle scenes (ui_eyes.c): no scene now, nobody's there. */
bool pet_quiet(void);
/** A scene that suits the mood, or NULL for any. */
const char *pet_idle_scene(void);
/** The wait between idle scenes, times this: shorter when lively. */
float pet_idle_pace(void);
/** The eyes face's colour, `base` tinted by the mood. */
uint32_t pet_tint(uint32_t base);
/** The eyes face's lids lower when tired, and it smiles when happy. */
void pet_pose(float *droop, float *smile);

#ifdef __cplusplus
}
#endif
