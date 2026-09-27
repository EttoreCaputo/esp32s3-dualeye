# Firmware changelog

What changes on the board with each firmware version. The desktop app shows
the entries newer than the firmware it finds on the board when it offers an
update, so write them for the person plugging the DualEye in.

Keep one `## <version>` section per release, newest first; the version is the
one in `version.txt`, which ESP-IDF builds into the image.

## 0.3.0

- Each screen can be turned by 90°, 180° or 270°, for a board that sits another way round. The board remembers it and boots turned.

## 0.2.0

- Tells the app which firmware it runs, so the app can offer updates like this one.
- New Plus and Bar faces: the classic face with a RAM or VRAM bar.
- New Claude and Clawd faces: Claude Code's 5-hour and weekly limits, and Clawd showing whether Claude is working.

## 0.1.0

- Firmware from before versioning: the Classic and Rings faces.
