#!/bin/sh
# once, on the mac: builds an lgpl shared ffmpeg 9.0 with videotoolbox into spike/third_party/ffmpeg (ignored by git).
#   sh spike/mac/setup.sh
# needs apple's command line tools. the source and build tree go to /private/tmp (about 0.5 GB); the install is about 40 MB.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
prefix="$here/third_party/ffmpeg"
src=/private/tmp/eyepiece-ffmpeg-src
mkdir -p "$here/third_party"
[ -d "$src" ] || git clone --quiet --depth 1 --branch release/9.0 https://github.com/FFmpeg/FFmpeg.git "$src"
cd "$src"
git log -1 --format='ffmpeg source %h %cs'
# no gpl or nonfree parts, nothing picked up from the machine by accident, only the libraries the player links
./configure --prefix="$prefix" --enable-shared --disable-static --disable-programs --disable-doc --disable-debug \
  --disable-autodetect --enable-videotoolbox --enable-zlib \
  --disable-avdevice --disable-avfilter --disable-swscale > configure.out 2>&1
grep -E '^License' configure.out
# niced and on half the cores: this mac is somebody's desk machine
nice -n 10 make -j4 > make.out 2>&1
make install > install.out 2>&1
ls "$prefix/lib" | grep -c dylib
