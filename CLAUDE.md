# DualEye PC Monitor

## Rule: every change bumps the version and gets built

Whenever you change the firmware (`main/`, `sdkconfig*`, `partitions.csv`) or anything the board runs:

> **Always produce a new ESP32-S3 firmware build with a higher version number.** No firmware change is done until `version.txt` has been bumped and `idf.py build merge-bin` has succeeded, which regenerates `build/merged-binary.bin`. This applies even to one-line fixes. Don't skip it or leave it for the user. If the build fails, fix it, or report the failure clearly.

1. **Bump `version.txt`** (patch by default, e.g. 1.4.1 → 1.4.2; minor for a new feature set when asked). Check `git log` first: if the current version has already been committed (released), it must go up. Bump once per piece of work, not on every edit within the same uncommitted change.
2. **Add a `## <version>` section at the top of `FIRMWARE_CHANGELOG.md`** with the changes, written for the person using the board. Never add new changes to a version that's already committed.
3. **Update the "firmware X.Y.Z or newer" mentions** for the new features (README, `docs/protocol.md`, the app's hints in `host/dualeye-app/src`) to the new version.
4. **Build everything**, so the app ships the new firmware:
   - firmware and the merged image the app embeds: `idf.py build merge-bin` (ESP-IDF 6.1; activate it with `source ~/.espressif/tools/activate_idf_v6.1.sh`)
   - host: `cargo build --workspace` in `host/` (it warns if `build/merged-binary.bin` is older than the firmware sources)
   - app UI: `npm run check` in `host/dualeye-app`
5. Run the tests: `cargo test --workspace` in `host/`.

## Rule: the app (software) version goes up too

The desktop app has its own version, separate from the firmware's. CI publishes the GitHub release as `v<app version>`, so if the app version stays the same, every merge just overwrites the same release. Whenever you change anything under `host/` or the firmware the app embeds, or anything else that ends up in a new build of the app:

1. **Bump the app version** (patch by default, e.g. 2.0.0 → 2.0.1), keeping all of these identical:
   - `host/dualeye-app/src-tauri/tauri.conf.json` (`version`: this names the release)
   - `host/dualeye-app/package.json`
   - `host/dualeye-app/src-tauri/Cargo.toml` (the package `version`, and the `version` on its `dualeye-core` dependency)
   - `host/dualeye-core/Cargo.toml`, `host/dualeye-cli/Cargo.toml`, `host/dualeye-power-helper/Cargo.toml`

   Check `git log` first. If the current app version is already committed, it must go up. Bump it once per piece of work.
2. **Rebuild** so the lock files pick up the new version: `cargo build --workspace` in `host/` (`Cargo.lock`) and `npm install` in `host/dualeye-app` (`package-lock.json`). Then run `npm run check` and `cargo test --workspace`.

A firmware change therefore bumps both versions: `version.txt` and the app version.

Report the new version(s) and the build results when done.
