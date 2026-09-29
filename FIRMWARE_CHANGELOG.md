# Firmware changelog

What changes on the board with each firmware version. The desktop app shows
the entries newer than the firmware it finds on the board when it offers an
update, so write them for the person plugging the DualEye in.

Keep one `## <version>` section per release, newest first; the version is the
one in `version.txt`, which ESP-IDF builds into the image.

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
