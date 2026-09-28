#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ES8311 (speaker, behind a PA) and ES7210 (mic ADC) share one I2S port in
 * full duplex and one I2C bus, same layout as Espressif's Box boards. Pins
 * from Waveshare's own xiaozhi-esp32 port for this board. */
#define BOARD_AUDIO_I2C_SCL_GPIO 10
#define BOARD_AUDIO_I2C_SDA_GPIO 11
#define BOARD_AUDIO_I2S_MCLK_GPIO 12
#define BOARD_AUDIO_I2S_BCLK_GPIO 13
#define BOARD_AUDIO_I2S_WS_GPIO 14
#define BOARD_AUDIO_I2S_DIN_GPIO 15  /* ES7210 -> ESP32 */
#define BOARD_AUDIO_I2S_DOUT_GPIO 16 /* ESP32 -> ES8311 */
#define BOARD_AUDIO_PA_GPIO 9

/** ESP-SR wants 16 kHz, so everything runs at it. */
#define BOARD_AUDIO_SAMPLE_RATE 16000
/** ES7210 TDM slots, read interleaved. Which one carries the mic and which
 * the speaker loopback (AEC reference) is what the M0 self-test finds out. */
#define BOARD_AUDIO_IN_CHANNELS 4

/** Bring up I2C, I2S and both codecs. Output starts muted. */
esp_err_t board_audio_init(void);

/** Blocking write of mono 16-bit samples to the speaker. */
esp_err_t board_audio_write(const int16_t *samples, size_t count);

/** Blocking read of `frames` frames of BOARD_AUDIO_IN_CHANNELS interleaved samples. */
esp_err_t board_audio_read(int16_t *frames_out, size_t frames);

/** 0..100. */
esp_err_t board_audio_set_volume(int volume);
esp_err_t board_audio_set_mute(bool mute);
/** Analog gain for every input channel, in dB (ES7210: 0..37.5). */
esp_err_t board_audio_set_in_gain(float db);

#ifdef __cplusplus
}
#endif
