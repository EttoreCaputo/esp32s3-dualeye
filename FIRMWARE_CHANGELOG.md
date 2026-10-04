# Firmware changelog

What changes on the board with each firmware version. The desktop app shows
the entries newer than the firmware it finds on the board when it offers an
update, so write them for the person plugging the DualEye in.

Keep one `## <version>` section per release, newest first; the version is the
one in `version.txt`, which ESP-IDF builds into the image.

## 1.3.2

- **Talking with music on.** The board tells your voice from music and noise much better, so it stops listening when you've finished instead of running on to the 12-second limit, and it waits a little longer when you pause mid-sentence. The app also pauses the music playing on your computer while you talk to it, and plays it again after.

## 1.3.1

- **The voice knows every face.** The board's list of what it can do had grown too long to reach the computer, so the voice only knew the first faces: "Alexa, metti la musica sul display destro" now finds the music face, and the eyes, timer, network, disk and battery ones too.

## 1.3.0

- **Music.** A new face shows what's playing on your computer: its cover over the whole screen, the position on a thin ring round the edge, the title and the artist. Paused, the cover dims and a pause sign shows; a track without a cover gets a record instead. "Alexa, pausa", "next song", "cosa sta suonando?" work too.
- **Eyes.** A new face is one big eye per screen that follows your mouse pointer. It blinks, glances about when the pointer stops, dozes off after a minute of stillness and wakes with a start when you move the mouse. Put it on both screens for a pair.

## 1.2.0

- **Timers, reminders and a pomodoro.** "Alexa, timer 10 minuti", "ricordami alle 17 di chiamare Marco", "start a pomodoro": the right screen shows a ring that empties and the time left, and the board chimes when it's up until you say "Alexa". A new **Timer** face keeps them on a screen all the time. Set them from the app's Timers tab too.
- **Network** and **Disk** faces, clearer: the speed's unit sits under it, upload below a divider, and the network rings show the speed on a fixed scale (a sixth of ring for each tenfold) instead of filling up whenever traffic is steady; the disk's reads and writes go one above the other, away from the ring.

## 1.1.0

- Any face on either screen: the CPU and GPU faces show whichever you pick, so both screens can show the GPU, or the CPU can go on the right.
- Three new faces: **Network** (download and upload speed), **Disk** (space used, reads and writes) and **Battery** (charge, charging, time left).
- **Image**: a picture or an animated GIF of your own on a screen, picked in the app. The board keeps it and shows it even with the app closed.
- A new flash layout makes room for the pictures: the app flashes the whole image, so update from the app. Your settings stay.

## 1.0.2

- The eyes now come out on their own: every minute or two, while nobody is talking, they open over the watch faces for a few seconds — a look around, a wink, a yawn, a roll of the eyes, a dizzy spin, hearts in their eyes and a dozen more — then close again. Turn it off in the app's Voice tab.

## 1.0.1

- Animated eyes: after "Alexa" the screens become two cartoon eyes that open, watch you while you talk, look up and think, smile and bob along as the board answers, and close when it's done. Turn them off in the app's Voice tab to get the ring back.

## 1.0.0

- Voice, finished: with voice on in the app, a small language model on your computer understands what you say, in Italian or English, and the board answers out loud.
- Follow-ups: after an answer the board keeps listening for a few seconds, so you can go on without saying "Alexa" again.
- Say "Alexa" while the board is talking to interrupt it: it stops and listens.
- When something goes wrong, or it didn't catch your words, the ring turns red and the board plays two short notes.

## 0.6.0

- Voice: say "Alexa" (or "Hi ESP", with the `set_wake_word` tool) and a ring lights round the screens. With voice on in the app, the board then sends what you say to your computer, which carries out simple commands and answers out loud through the board's speaker. Everything stays on your computer.
- The speaker's volume can be set from the app, and the board remembers it.
- The microphone can be muted (`set_mic`); the board remembers that too.
- A new flash layout makes room for the wake-word models: the app flashes the whole image, so update from the app.

## 0.4.0

- New connection to the app (protocol v2): it needs the app version that comes with this firmware. Boot messages and crash reports still show in the app's board console.
- The board remembers each screen's face and brightness as well as its rotation, and boots showing them.
- The `dualeye` CLI can control the board directly: change a face, turn a screen, set the brightness, or show a short message on the screens.

## 0.3.0

- Each screen can be turned by 90°, 180° or 270°, for a board that sits another way round. The board remembers it and boots turned.

## 0.2.0

- Tells the app which firmware it runs, so the app can offer updates like this one.
- New Plus and Bar faces: the classic face with a RAM or VRAM bar.
- New Claude and Clawd faces: Claude Code's 5-hour and weekly limits, and Clawd showing whether Claude is working.

## 0.1.0

- Firmware from before versioning: the Classic and Rings faces.
