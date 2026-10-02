#!/usr/bin/env bash
# Build dualeye-power-helper, the macOS app's root helper, where Tauri's
# `externalBin` wants it (see host/dualeye-app/src-tauri/tauri.macos.conf.json):
#
#   tools/build_power_helper.sh [TARGET_TRIPLE]
#
# TARGET_TRIPLE defaults to the one Tauri builds for (TAURI_ENV_TARGET_TRIPLE,
# set for its before-build commands), else the host's. `universal-apple-darwin`
# builds both architectures and joins them with lipo.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
target="${1:-${TAURI_ENV_TARGET_TRIPLE:-$(rustc -vV | sed -n 's/^host: //p')}}"
out="$root/host/dualeye-app/src-tauri/binaries"
mkdir -p "$out"

build() {
    cargo build --release --quiet --manifest-path "$root/host/Cargo.toml" -p dualeye-power-helper --target "$1"
    echo "$root/host/target/$1/release/dualeye-power-helper"
}

case "$target" in
    universal-apple-darwin)
        arm="$(build aarch64-apple-darwin)"
        intel="$(build x86_64-apple-darwin)"
        lipo -create -output "$out/dualeye-power-helper-$target" "$arm" "$intel"
        ;;
    *-apple-darwin)
        cp "$(build "$target")" "$out/dualeye-power-helper-$target"
        ;;
    *)
        echo "dualeye-power-helper is macOS only, not $target" >&2
        exit 1
        ;;
esac
echo "built $out/dualeye-power-helper-$target"
