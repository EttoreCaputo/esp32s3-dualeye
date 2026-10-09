#pragma once

/* What each screen shows and how: set by the host's tools, kept in NVS. */

#include <stdbool.h>
#include <stdint.h>

#include "board_display.h"
#include "esp_err.h"
#include "metrics_model.h"

#ifdef __cplusplus
extern "C" {
#endif

#define BOARD_BRIGHTNESS_DEFAULT 100
#define BOARD_VOLUME_DEFAULT 60

typedef struct {
    metrics_face_t face[BOARD_LCD_COUNT];
    /* Whose metrics the screen's classic, rings, plus and bar faces show. */
    metrics_source_t source[BOARD_LCD_COUNT];
    /* Extra clockwise turn on top of the DualEye mounting: 0, 90, 180, 270. */
    uint16_t rot[BOARD_LCD_COUNT];
    uint8_t brightness[BOARD_LCD_COUNT];
    /* The wake word isn't listened for. */
    bool mic_muted;
    /* voice_wake_words() id; empty for the default. */
    char wake_word[16];
    /* Speaker, 0..100. */
    uint8_t volume;
    /* Animated eyes during a conversation, rather than the ring. */
    bool eyes;
    /* The eyes play a short scene now and then while nobody is talking. */
    bool idle_eyes;
    /* A short chime as the board starts listening after the wake word. */
    bool wake_sound;
    /* The pet's own sounds: in the eyes' scenes, hello, mute... */
    bool pet_sounds;
    /* The pet reacts to what happens on the computer (pet.c). */
    bool pet_reactions;
} board_settings_t;

/** Open NVS and load the saved settings (defaults where none). */
void board_settings_init(void);

void board_settings_get(board_settings_t *out);

/* Each setter saves to NVS. Screens are UI_SCREEN_*. */
esp_err_t board_settings_set_face(int screen, metrics_face_t face);
esp_err_t board_settings_set_source(int screen, metrics_source_t source);
esp_err_t board_settings_set_rotation(int screen, uint16_t degrees);
/** Also sets the backlight right away. */
esp_err_t board_settings_set_brightness(int screen, uint8_t percent);
esp_err_t board_settings_set_mic_muted(bool muted);
esp_err_t board_settings_set_wake_word(const char *id);
/** Also sets the speaker's volume right away. */
esp_err_t board_settings_set_volume(uint8_t percent);
esp_err_t board_settings_set_eyes(bool on);
esp_err_t board_settings_set_idle_eyes(bool on);
esp_err_t board_settings_set_wake_sound(bool on);
esp_err_t board_settings_set_pet_sounds(bool on);
esp_err_t board_settings_set_pet_reactions(bool on);

#ifdef __cplusplus
}
#endif
