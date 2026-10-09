# DualEye PC Monitor

## Rule: every change bumps the version and gets built

Whenever you change the firmware (`main/`, `sdkconfig*`, `partitions.csv`) or anything the board runs:

1. **Bump `version.txt`** (patch by default, e.g. 1.4.1 → 1.4.2; minor for a new feature set when asked). Check `git log` first: if the current version has already been committed (released), it must go up. Bump once per piece of work, not on every edit within the same uncommitted change.
2. **Add a `## <version>` section at the top of `FIRMWARE_CHANGELOG.md`** with the changes, written for the person using the board. Never add new changes to a version that's already committed.
3. **Update the "firmware X.Y.Z or newer" mentions** for the new features (README, `docs/protocol.md`, the app's hints in `host/dualeye-app/src`) to the new version.
4. **Build everything**, so the app ships the new firmware:
   - firmware and the merged image the app embeds: `idf.py build merge-bin` (ESP-IDF 6.1; activate it with `source ~/.espressif/tools/activate_idf_v6.1.sh`)
   - host: `cargo build --workspace` in `host/` (it warns if `build/merged-binary.bin` is older than the firmware sources)
   - app UI: `npm run check` in `host/dualeye-app`
5. Run the tests: `cargo test --workspace` in `host/`.

Report the new version and the build results when done.
