#!/usr/bin/env bash
# Build whisper.cpp's whisper-server and llama.cpp's llama-server for the
# desktop app's bundle (see host/dualeye-app/src-tauri/tauri.sidecars.conf.json).
#
#   tools/build_sidecars.sh [TARGET_TRIPLE]
#
# TARGET_TRIPLE defaults to the host's (`rustc -vV`). The binaries go to
# host/dualeye-app/src-tauri/binaries/<name>-<triple>[.exe], where Tauri's
# `externalBin` wants them, and the two projects' licenses next to them in
# binaries/licenses/. Each binary is linked statically (no libggml,
# libllama or libwhisper beside it), without OpenMP and without OpenSSL (the
# servers only listen on 127.0.0.1 and download nothing), so it runs anywhere
# the app does:
#
#   - macOS Apple silicon: Metal, with its shaders built in.
#   - macOS Intel, Windows, Linux: the CPU (AVX2). An NVIDIA card needs a CUDA
#     build of llama.cpp, pointed at with DUALEYE_LLAMA_SERVER.
#
# Needs git, cmake and a C++ compiler (Xcode, MSVC, gcc).

set -euo pipefail

LLAMA_CPP=b11146
WHISPER_CPP=v1.9.4

root="$(cd "$(dirname "$0")/.." && pwd)"
target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
out="$root/host/dualeye-app/src-tauri/binaries"
work="${SIDECAR_WORK:-$root/build/sidecars}"
exe=""
[[ "$target" == *windows* ]] && exe=".exe"

cmake_args=(
    -DCMAKE_BUILD_TYPE=Release
    -DBUILD_SHARED_LIBS=OFF
    -DGGML_NATIVE=OFF
    -DGGML_OPENMP=OFF
    # Windows: the C runtime inside, not a vcruntime DLL.
    -DCMAKE_POLICY_DEFAULT_CMP0091=NEW
    -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded
)
case "$target" in
    aarch64-apple-darwin) cmake_args+=(-DCMAKE_OSX_ARCHITECTURES=arm64 -DGGML_METAL=ON -DGGML_METAL_EMBED_LIBRARY=ON) ;;
    x86_64-apple-darwin) cmake_args+=(-DCMAKE_OSX_ARCHITECTURES=x86_64 -DGGML_METAL=OFF) ;;
    *) ;;
esac
[[ "$target" == *apple-darwin ]] && cmake_args+=(-DCMAKE_OSX_DEPLOYMENT_TARGET=11.0)

# fetch NAME TAG: a shallow clone of ggml-org/NAME at TAG in $work/NAME.
fetch() {
    local dir="$work/$1"
    if [[ "$(git -C "$dir" describe --tags --exact-match 2>/dev/null)" != "$2" ]]; then
        rm -rf "$dir"
        git clone --quiet --depth 1 --branch "$2" "https://github.com/ggml-org/$1.git" "$dir"
    fi
}

# build NAME TARGET [CMAKE ARGS...]: the binary's path on stdout.
build() {
    local name="$1" bin="$2"
    shift 2
    local dir="$work/$name"
    cmake -S "$dir" -B "$dir/build-$target" "${cmake_args[@]}" "$@" >&2
    cmake --build "$dir/build-$target" --config Release --target "$bin" -j "${JOBS:-4}" >&2
    # Single-config generators put it in bin/, Visual Studio in bin/Release/.
    for p in "$dir/build-$target/bin/$bin$exe" "$dir/build-$target/bin/Release/$bin$exe"; do
        if [[ -f "$p" ]]; then
            echo "$p"
            return
        fi
    done
    echo "$bin$exe not found after the build" >&2
    exit 1
}

mkdir -p "$work" "$out/licenses"
fetch llama.cpp "$LLAMA_CPP"
fetch whisper.cpp "$WHISPER_CPP"

llama="$(build llama.cpp llama-server -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF -DLLAMA_BUILD_SERVER=ON -DLLAMA_CURL=OFF -DLLAMA_OPENSSL=OFF)"
whisper="$(build whisper.cpp whisper-server -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_EXAMPLES=ON -DWHISPER_BUILD_SERVER=ON -DWHISPER_SDL2=OFF -DWHISPER_CURL=OFF)"

cp "$llama" "$out/llama-server-$target$exe"
cp "$whisper" "$out/whisper-server-$target$exe"
cp "$work/llama.cpp/LICENSE" "$out/licenses/llama.cpp-LICENSE.txt"
cp "$work/whisper.cpp/LICENSE" "$out/licenses/whisper.cpp-LICENSE.txt"
printf 'llama.cpp %s\nwhisper.cpp %s\n' "$LLAMA_CPP" "$WHISPER_CPP" > "$out/licenses/VERSIONS.txt"

ls -l "$out"/*-"$target$exe"
# Nothing outside the OS: no Homebrew or distribution libraries.
case "$target" in
    *apple-darwin) deps="$(otool -L "$out"/*-"$target" | grep -v -e ':$' -e '^\s*/System/' -e '^\s*/usr/lib/' || true)" ;;
    *linux*) deps="$(ldd "$out"/*-"$target" | grep -v -E 'linux-vdso|ld-linux|lib(c|m|pthread|dl|rt|stdc\+\+|gcc_s)\.so' | grep '=>' || true)" ;;
    *) deps="" ;;
esac
if [[ -n "$deps" ]]; then
    echo "linked against libraries the app doesn't ship:" >&2
    echo "$deps" >&2
    exit 1
fi
# A quick sign of life (on the machine that built it; not when cross-building).
if [[ "$target" == "$(rustc -vV | sed -n 's/^host: //p')" ]]; then
    "$out/llama-server-$target$exe" --version 2>&1 | tail -1
fi
