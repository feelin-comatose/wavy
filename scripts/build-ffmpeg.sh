#!/usr/bin/env bash
# Минимальный ffmpeg для Wavy: только то, что не умеет встроенный декодер — видео HEVC/H.265 (в т.ч. 10 бит),
# 10-битное H.264, VP9, AV1, MPEG-4 и звук Opus, AC3/E-AC3, DTS, TrueHD, — плюс аппаратное декодирование
# видеокартой (D3D11VA/DXVA2) на Windows. Получается один ffmpeg(.exe) на несколько МБ вместо готовых сборок на 80–180 МБ.
#
#   scripts/build-ffmpeg.sh windows <папка>   — ffmpeg.exe (кросс-сборка mingw, так собирает CI)
#   scripts/build-ffmpeg.sh linux <папка>     — ffmpeg для проверок на Linux
#
# Исходники: переменная SRC (распакованный ffmpeg-$VER) или скачиваются с ffmpeg.org в WORK.
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
cd "$BUILD"

COMMON=(
  --disable-everything --disable-autodetect --disable-doc --disable-debug --disable-network
  --disable-ffplay --disable-ffprobe --enable-ffmpeg
  --disable-avdevice --disable-postproc
  --enable-swscale --enable-swresample --enable-avfilter
  --enable-protocol=file,pipe
  --enable-demuxer=matroska,mov,avi,mpegts,flv,ogg
  --enable-decoder=hevc,h264,vp8,vp9,av1,mpeg4,mpeg2video,msmpeg4v3
  --enable-decoder=aac,aac_latm,ac3,eac3,dca,truehd,mlp,opus,vorbis,flac,mp3,mp2,alac,pcm_s16le,pcm_s24le,pcm_s32le,pcm_f32le
  --enable-parser=hevc,h264,vp8,vp9,av1,mpeg4video,mpegvideo,aac,aac_latm,ac3,dca,mlp,opus,vorbis,flac,mpegaudio
  --enable-filter=scale,format,showinfo,null,anull,aresample,aformat
  --enable-encoder=rawvideo,pcm_f32le
  --enable-muxer=rawvideo,pcm_f32le
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
