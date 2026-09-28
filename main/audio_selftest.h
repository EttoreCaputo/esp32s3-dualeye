#pragma once

#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/** Bring up the codecs and the self-test task. Failing here leaves the rest of
 * the firmware running without audio. */
void audio_selftest_start(void);

/** Queue one self-test command, from the `debug/audio` JSON-RPC method (M0
 * bring-up aid; tools/audio_selftest.py is the host side). Output goes to the
 * host as `log` lines:
 *
 *   tone [hz ms]          chime, or a sine of hz for ms
 *   rec S [tone|beep]     record S seconds of all input channels (1 kHz sine
 *                         meanwhile with "tone", a "speak now" beep first with "beep"),
 *                         then dump them as AUD:BEGIN / AUD:D <base64> / AUD:END
 *   play [ch]             play channel ch (default 0) of the last recording
 *   vol N                 speaker volume 0..100
 *   gain DB               input gain for every channel
 *   stats                 heap and per-task CPU over one second
 *
 * Every command ends with a line "AUD:OK <cmd>" or "AUD:ERR <reason>". Returns
 * true once the command is accepted (or answered with AUD:ERR). */
bool audio_selftest_command(const char *cmd);

#ifdef __cplusplus
}
#endif
