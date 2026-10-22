#!/bin/sh
# Builds the small FFmpeg the Android build plays the game's movies with
# (the front end's backdrop and music, the cut-scenes): Bink in, raw RGBA
# pictures and 16-bit PCM sound out through pipes, as crates/m2-alpha's
# movie.rs asks of it. LGPL only (no GPL or non-free parts).
#
#   sh android/build-ffmpeg.sh <ffmpeg source dir>
#
# from Git Bash (or any POSIX shell), with the NDK at $ANDROID_NDK_ROOT. The
# program lands in android/target/runtime-libs/arm64-v8a/libffmpeg.so,
# which build-apk.ps1 packs into the APK (Android unpacks lib*.so files to
# the one folder an app may run programs from). Source: an FFmpeg release
# tarball (built with 7.1.1), checked against its signature.
set -e
SRC=${1:?usage: build-ffmpeg.sh <ffmpeg source dir>}
HERE=$(cd "$(dirname "$0")" && pwd)
NDK=${ANDROID_NDK_ROOT:?set ANDROID_NDK_ROOT}
NDK=$(cd "$NDK" && pwd)
case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) HOST=windows-x86_64 EXE=.exe ;;
    Darwin) HOST=darwin-x86_64 EXE= ;;
    *) HOST=linux-x86_64 EXE= ;;
esac
TC=$NDK/toolchains/llvm/prebuilt/$HOST
BIN=$TC/bin
API=26
TARGET="--target=aarch64-linux-android$API"
# configure writes test programs to TMPDIR; a Windows path there breaks it.
export TMPDIR="$HERE/target/ffmpeg-tmp"
mkdir -p "$TMPDIR"
MAKE=make
command -v make > /dev/null || MAKE=$NDK/prebuilt/$HOST/bin/make$EXE
cd "$SRC"
# The host compiler is checked but builds nothing here (no hardcoded
# tables): the NDK's clang, against Android's headers, passes the check.
./configure \
    --enable-cross-compile --target-os=android --arch=aarch64 --cpu=armv8-a \
    --cc="$BIN/clang$EXE" --cxx="$BIN/clang++$EXE" --ld="$BIN/clang$EXE" \
    --ar="$BIN/llvm-ar$EXE" --nm="$BIN/llvm-nm$EXE" --ranlib="$BIN/llvm-ranlib$EXE" \
    --strip="$BIN/llvm-strip$EXE" --sysroot="$TC/sysroot" \
    --extra-cflags="$TARGET -O2" --extra-ldflags="$TARGET -pie" \
    --host-cc="$BIN/clang$EXE" \
    --host-cflags="$TARGET --sysroot=$TC/sysroot" \
    --host-ldflags="$TARGET --sysroot=$TC/sysroot" \
    --pkg-config=false --disable-autodetect --disable-everything \
    --disable-doc --disable-network --disable-debug --disable-ffplay --disable-ffprobe \
    --enable-static --disable-shared --enable-ffmpeg \
    --enable-avcodec --enable-avformat --enable-avfilter --enable-swscale --enable-swresample \
    --enable-demuxer=bink --enable-decoder=bink,binkaudio_rdft,binkaudio_dct \
    --enable-encoder=rawvideo,pcm_s16le --enable-muxer=rawvideo,pcm_s16le \
    --enable-protocol=file,pipe \
    --enable-filter=amerge,pan,aresample,aformat,format,scale,null,anull,copy,acopy
"$MAKE" -j8 ffmpeg
OUT="$HERE/target/runtime-libs/arm64-v8a"
mkdir -p "$OUT"
"$BIN/llvm-strip$EXE" -o "$OUT/libffmpeg.so" ffmpeg
echo "built $OUT/libffmpeg.so"
