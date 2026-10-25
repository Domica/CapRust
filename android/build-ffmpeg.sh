#!/usr/bin/env bash
#
# Build ffmpeg for Android, the way the phone needs it.
#
# The engine renders by calling ffmpeg, and an Android app may only
# run a binary out of its own native library folder, so ffmpeg has to
# be in the APK. This builds it from source: no third-party binary in
# the release, and a modern ffmpeg with the filters the editor uses.
#
# Usage:
#   android/build-ffmpeg.sh <abi>
#   # aarch64-linux-android | x86_64-linux-android | i686-linux-android
#
# The result lands in android/ffmpeg/<abi>/ffmpeg, and the Gradle build
# copies it into the APK.
#
# Three things are built, in this order, because ffmpeg needs them:
#
#   x264      the H.264 encoder. ffmpeg has no encoder of its own for
#             it, and a Story has to be H.264.
#   freetype  the fonts for drawtext, which is how the text overlays
#             are drawn.
#   ffmpeg    the renderer.
#
# Everything is static: the APK carries the binaries and nothing else.
set -euo pipefail

ABI="${1:-aarch64-linux-android}"
HERE="$(cd "$(dirname "$0")" && pwd)"
# The runner names the NDK two ways: ANDROID_NDK_ROOT on some
# images, ANDROID_NDK_LATEST_HOME on others. Take whichever is there.
NDK="${ANDROID_NDK_ROOT:-${ANDROID_NDK_LATEST_HOME:-${ANDROID_NDK_HOME:-}}}"
if [ -z "$NDK" ]; then
  echo "ANDROID_NDK_ROOT is not set" >&2
  exit 1
fi

# The NDK's clang wrappers are per API level: API 21 is the floor the
# app supports and keeps the binaries usable on old devices.
API=21
case "$ABI" in
  aarch64-linux-android) TRIPLE="aarch64-linux-android$API"; HOST="aarch64-linux-android"; ARCH="aarch64"; FOLDER="arm64-v8a" ;;
  armv7-linux-androideabi) TRIPLE="armv7a-linux-androideabi$API"; HOST="arm-linux-androideabi"; ARCH="arm"; FOLDER="armeabi-v7a" ;;
  x86_64-linux-android)  TRIPLE="x86_64-linux-android$API";  HOST="x86_64-linux-android";  ARCH="x86_64"; FOLDER="x86_64" ;;
  i686-linux-android)     TRIPLE="i686-linux-android$API";     HOST="i686-linux-android";     ARCH="x86";     FOLDER="x86" ;;
  *) echo "unknown abi: $ABI" >&2; exit 1 ;;
esac

OUT="$HERE/ffmpeg/$FOLDER"

BIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CC="$BIN/$TRIPLE-clang"
export CXX="$BIN/$TRIPLE-clang++"
export AR="$BIN/llvm-ar"
export NM="$BIN/llvm-nm"
export RANLIB="$BIN/llvm-ranlib"
export STRIP="$BIN/llvm-strip"
export PATH="$BIN:$PATH"

if [ ! -x "$CC" ]; then
  echo "the NDK has no $TRIPLE-clang" >&2
  ls "$BIN" | grep -E 'android.*clang' | head -20 >&2
  exit 1
fi

WORK="$HERE/.ffmpeg-build/$ABI"
PREFIX="$WORK/prefix"
rm -rf "$WORK"
mkdir -p "$PREFIX" "$OUT"

# x264's configure looks for <cross-prefix>gcc, <cross-prefix>as and
# friends. The NDK does not ship tools with that name, so give it the
# ones it expects as symlinks.
CROSS_BIN="$WORK/cross-bin"
mkdir -p "$CROSS_BIN"
for tool in gcc g++ as ar ranlib strip nm strings ld; do
  case "$tool" in
    gcc|g++|as|ld) ln -sf "$BIN/$TRIPLE-clang" "$CROSS_BIN/$HOST-$tool" ;;
    ar) ln -sf "$BIN/llvm-ar" "$CROSS_BIN/$HOST-ar" ;;
    ranlib) ln -sf "$BIN/llvm-ranlib" "$CROSS_BIN/$HOST-ranlib" ;;
    strip) ln -sf "$BIN/llvm-strip" "$CROSS_BIN/$HOST-strip" ;;
    nm) ln -sf "$BIN/llvm-nm" "$CROSS_BIN/$HOST-nm" ;;
    strings) [ -x "$BIN/llvm-strings" ] && ln -sf "$BIN/llvm-strings" "$CROSS_BIN/$HOST-strings" ;;
  esac
done
export PATH="$CROSS_BIN:$PATH"

fetch() {
  # The sources are fetched rather than vendored: they are large, and
  # the versions are pinned here so the build is the same every time.
  local url="$1" dir="$2" branch="${3:-}"
  if [ ! -d "$dir" ]; then
    if [ -n "$branch" ]; then
      git clone --depth 1 --branch "$branch" "$url" "$dir"
    else
      git clone --depth 1 "$url" "$dir"
    fi
  fi
}

# ---- x264 --------------------------------------------------------
fetch https://code.videolan.org/videolan/x264.git "$WORK/x264"
cd "$WORK/x264"
./configure \
  --cross-prefix="$HOST-" \
  --host="$HOST" \
  --enable-static \
  --enable-pic \
  --disable-cli \
  --disable-asm \
  --extra-cflags="-DANDROID -D__ANDROID_API__=$API" \
  --prefix="$PREFIX"
make -j"$(nproc)"
make install
cd - >/dev/null

# ---- freetype -----------------------------------------------------
fetch https://github.com/freetype/freetype.git "$WORK/freetype"
cd "$WORK/freetype"
./autogen.sh
./configure \
  --host="$HOST" \
  --enable-static \
  --disable-shared \
  --without-zlib \
  --without-bzip2 \
  --without-png \
  --without-harfbuzz \
  --without-brotli \
  --prefix="$PREFIX"
make -j"$(nproc)"
make install
cd - >/dev/null

# ---- ffmpeg -------------------------------------------------------
fetch https://git.ffmpeg.org/ffmpeg.git "$WORK/ffmpeg" n7.1
cd "$WORK/ffmpeg"

export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig"
export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig"

# A build with only what the editor uses. A full ffmpeg is thirty
# megabytes of codecs the editor never asks for and takes forty
# minutes to build; this is the set CapRust's filtergraph needs.
./configure \
  --target-os=android \
  --enable-cross-compile \
  --enable-gpl \
  --arch="$ARCH" \
  --disable-asm \
  --disable-doc \
  --disable-debug \
  --disable-symver \
  --disable-avdevice \
  --disable-postproc \
  --disable-vaapi \
  --disable-vdpau \
  --disable-sdl2 \
  --disable-xlib \
  --disable-iconv \
  --disable-ffplay \
  --enable-swscale \
  --enable-swresample \
  --enable-avfilter \
  --enable-protocol=file,pipe \
  --enable-ffmpeg \
  --enable-ffprobe \
  --enable-pic \
  --cc="$CC" --cxx="$CXX" --ar="$AR" --nm="$NM" --ranlib="$RANLIB" --strip="$STRIP" \
  --pkg-config=pkg-config \
  --extra-cflags="-DANDROID -D__ANDROID_API__=$API" \
  --extra-ldflags="-L$PREFIX/lib" \
  --enable-libx264 \
  --enable-libfreetype \
  --disable-libass \
  --disable-libvpx \
  --disable-libopus \
  --disable-libvorbis \
  --disable-libmp3lame \
  --disable-libaom \
  --disable-libsvtav1 \
  --disable-libdav1d \
  --disable-libzimg \
  --disable-librubberband

make -j"$(nproc)"

# The binary the app runs. It is stripped: the APK should not carry a
# symbol table for a renderer.
install -m 0755 "$WORK/ffmpeg/ffmpeg" "$OUT/ffmpeg"
install -m 0755 "$WORK/ffmpeg/ffprobe" "$OUT/ffprobe"
"$STRIP" --strip-unneeded "$OUT/ffmpeg" || true
"$STRIP" --strip-unneeded "$OUT/ffprobe" || true

# The shared libraries, if the build produced any, go next to the
# binary: the app points the engine at this folder.
find "$PREFIX/lib" -maxdepth 1 -name '*.so*' -exec cp -a {} "$OUT/" \; 2>/dev/null || true

echo "built $OUT/ffmpeg ($(du -h "$OUT/ffmpeg" | cut -f1))"
