#pragma once

#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/** Lines starting with this prefix are audio self-test commands (M0, temporary
 * until protocol v2). See tools/audio_selftest.py for the host side. */
#define AUDIO_SELFTEST_PREFIX "!audio"

/** Bring up the codecs and the self-test task. Failing here leaves the rest of
 * the firmware running without audio. */
void audio_selftest_start(void);

/** Handle one command line (with AUDIO_SELFTEST_PREFIX). Returns false if it isn't one.
 *
 *   !audio tone [hz ms]   chime, or a sine of hz for ms
 *   !audio rec S [tone|beep]  record S seconds of all input channels (1 kHz sine
 *                         meanwhile with "tone", a "speak now" beep first with "beep"),
 *                         then dump them as AUD:BEGIN / AUD:D <base64> / AUD:END
 *   !audio play [ch]      play channel ch (default 0) of the last recording
 *   !audio vol N          speaker volume 0..100
 *   !audio gain DB        input gain for every channel
 *   !audio stats          heap and per-task CPU over one second
 *
 * Every command ends with a line "AUD:OK <cmd>" or "AUD:ERR <reason>". */
bool audio_selftest_command(const char *line);

#ifdef __cplusplus
}
#endif
