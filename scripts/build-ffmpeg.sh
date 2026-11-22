#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PREFIX="${1:-$ROOT/target/ffmpeg}"
VERSION=7.1.5
SHA256=de668509caf9e35e3cd162473441fdb29538c6d96ed080292b3cf9e6fc5d558f
TARBALL="ffmpeg-${VERSION}.tar.xz"
SRCDIR="$ROOT/target/ffmpeg-src-${VERSION}"
BUILD="$ROOT/target/ffmpeg-build-${VERSION}"
mkdir -p "$ROOT/target"
cd "$ROOT/target"
if [[ ! -f "$TARBALL" ]]; then
  curl -fL "https://ffmpeg.org/releases/$TARBALL" -o "$TARBALL"
fi
echo "$SHA256  $TARBALL" | sha256sum -c -
rm -rf "$SRCDIR" "$BUILD"
mkdir -p "$BUILD"
tar -xJf "$TARBALL" -C "$ROOT/target"
mv "$ROOT/target/ffmpeg-${VERSION}" "$SRCDIR"
cd "$BUILD"
"$SRCDIR/configure" \
  --prefix="$PREFIX" \
  --disable-everything \
  --disable-programs \
  --disable-doc \
  --disable-network \
  --disable-autodetect \
  --disable-devices \
  --disable-filters \
  --disable-outdevs \
  --disable-indevs \
  --arch=x86_64 \
  --target-os=linux \
  --enable-decoder=h264 \
  --enable-parser=h264 \
  --enable-swscale \
  --enable-pic \
  --enable-static \
  --disable-shared
MAKE="${MAKE:-make}"
"$MAKE" -j"$(nproc)"
"$MAKE" install