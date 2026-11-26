#!/usr/bin/env bash
# Minimal ffmpeg for Wavy: only what the built-in decoder can't do — HEVC/H.265 video (incl. 10-bit),
# 10-bit H.264, VP9, AV1, MPEG-4 and Opus, AC3/E-AC3, DTS, TrueHD audio — plus hardware decoding
# on the GPU (D3D11VA/DXVA2) on Windows, and an MP3 encoder (LAME) for playlist export. The result is a single ffmpeg(.exe) of a few MB instead of ready-made 80–180 MB builds.
#
#   scripts/build-ffmpeg.sh windows <folder>   — ffmpeg.exe (mingw cross-build, the way CI builds it)
#   scripts/build-ffmpeg.sh linux <folder>     — ffmpeg for checks on Linux
#
# Sources: the SRC variable (an unpacked ffmpeg-$VER) or downloaded from ffmpeg.org into WORK.
set -euo pipefail
TARGET=${1:-windows}
OUT=$(realpath -m "${2:-dist}")
VER=7.1.2
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$OUT" "$WORK"
if [ -z "${SRC:-}" ]; then
  SRC="$WORK/ffmpeg-$VER"
  if [ ! -d "$SRC" ]; then
    curl -fsSL -o "$WORK/ffmpeg.tar.xz" "https://ffmpeg.org/releases/ffmpeg-$VER.tar.xz"
    tar xJf "$WORK/ffmpeg.tar.xz" -C "$WORK"
  fi
fi
BUILD="$WORK/build-$TARGET"
mkdir -p "$BUILD"

# LAME — MP3 encoder (song export), built statically and linked into ffmpeg
LAME_VER=3.100
LAME_SHA=ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e
LAME_SRC="$WORK/lame-$LAME_VER"
LAME_OUT="$WORK/lame-$TARGET"
if [ ! -f "$LAME_OUT/lib/libmp3lame.a" ]; then
  if [ ! -d "$LAME_SRC" ]; then
    curl -fsSL -o "$WORK/lame.tar.gz" "https://downloads.sourceforge.net/project/lame/lame/$LAME_VER/lame-$LAME_VER.tar.gz"
    echo "$LAME_SHA  $WORK/lame.tar.gz" | sha256sum -c -
    tar xzf "$WORK/lame.tar.gz" -C "$WORK"
  fi
  mkdir -p "$WORK/lame-build-$TARGET"
  ( cd "$WORK/lame-build-$TARGET"
    HOST=(); [ "$TARGET" = windows ] && HOST=(--host=x86_64-w64-mingw32)
    "$LAME_SRC/configure" "${HOST[@]}" --prefix="$LAME_OUT" --enable-static --disable-shared --disable-frontend \
      --disable-decoder --disable-gtktest --disable-analyzer-hooks >/dev/null
    make -j"$(nproc)" >/dev/null && make install >/dev/null )
fi
cd "$BUILD"

COMMON=(
  --disable-everything --disable-autodetect --disable-doc --disable-debug --disable-network
  --disable-ffplay --disable-ffprobe --enable-ffmpeg
  --disable-avdevice --disable-postproc
  --enable-swscale --enable-swresample --enable-avfilter
  --enable-protocol=file,pipe
  --enable-demuxer=matroska,mov,avi,mpegts,flv,ogg,mp3,flac,wav,aac,aiff
  --enable-decoder=hevc,h264,vp8,vp9,av1,mpeg4,mpeg2video,msmpeg4v3
  --enable-decoder=aac,aac_latm,ac3,eac3,dca,truehd,mlp,opus,vorbis,flac,mp3,mp3float,mp2,alac,pcm_s16le,pcm_s24le,pcm_s32le,pcm_f32le,pcm_s16be,pcm_s24be
  --enable-parser=hevc,h264,vp8,vp9,av1,mpeg4video,mpegvideo,aac,aac_latm,ac3,dca,mlp,opus,vorbis,flac,mpegaudio
  --enable-filter=scale,format,showinfo,null,anull,aresample,aformat
  --enable-encoder=rawvideo,pcm_f32le,libmp3lame
  --enable-muxer=rawvideo,pcm_f32le,mp3
  --enable-libmp3lame --extra-cflags=-I$WORK/lame-$TARGET/include --extra-ldflags=-L$WORK/lame-$TARGET/lib
  --enable-small
)

case "$TARGET" in
  windows)
    HW=hevc_d3d11va,hevc_d3d11va2,hevc_dxva2,h264_d3d11va,h264_d3d11va2,h264_dxva2,vp9_d3d11va,vp9_d3d11va2,vp9_dxva2,av1_d3d11va,av1_d3d11va2,av1_dxva2
    "$SRC/configure" "${COMMON[@]}" --target-os=mingw32 --arch=x86_64 --cross-prefix=x86_64-w64-mingw32- \
      --enable-d3d11va --enable-dxva2 --enable-hwaccel=$HW --enable-w32threads \
      --extra-ldflags=-static --pkg-config=false
    BIN=ffmpeg.exe; STRIP=x86_64-w64-mingw32-strip ;;
  linux)
    "$SRC/configure" "${COMMON[@]}" --enable-pthreads --pkg-config=false
    BIN=ffmpeg; STRIP=strip ;;
  *) echo "неизвестная цель $TARGET"; exit 1 ;;
esac
make -j"$(nproc)" "$BIN" >/dev/null
$STRIP "$BIN"
cp "$BIN" "$OUT/"
ls -la "$OUT/$BIN"
