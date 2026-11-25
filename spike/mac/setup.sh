#!/bin/sh
# once, on the mac: builds an lgpl shared ffmpeg 9.0 with videotoolbox into spike/third_party/ffmpeg (ignored by git),
# for macos 13 and later ($MACOSX_DEPLOYMENT_TARGET to change it): built without it, the libraries ask for the build
# machine's own macos, and the released app would open on nothing older.
#   sh spike/mac/setup.sh
# needs apple's command line tools. the source and build tree go to /private/tmp (about 0.5 GB); the install is about 40 MB.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
prefix="$here/third_party/ffmpeg"
min=${MACOSX_DEPLOYMENT_TARGET:-13.0}
# a tree of its own for each minimum: make does not rebuild what was compiled for another
src=/private/tmp/eyepiece-ffmpeg-src-$min
branch=release/9.0
mkdir -p "$here/third_party"
[ -d "$src" ] || git clone --quiet --depth 1 --branch $branch https://github.com/FFmpeg/FFmpeg.git "$src"
cd "$src"
git log -1 --format='ffmpeg source %h %cs'
# no gpl or nonfree parts, nothing picked up from the machine by accident, only the libraries the player links
./configure --prefix="$prefix" --enable-shared --disable-static --disable-programs --disable-doc --disable-debug \
  --disable-autodetect --enable-videotoolbox --enable-zlib \
  --disable-avdevice --disable-avfilter --disable-swscale \
  --extra-cflags="-mmacosx-version-min=$min" --extra-ldflags="-mmacosx-version-min=$min" > configure.out 2>&1
grep -E '^License' configure.out
# niced and on half the cores: this mac is somebody's desk machine
nice -n 10 make -j4 > make.out 2>&1
make install > install.out 2>&1
# what the notices say it was built from (scripts/notices.ps1 -Target aarch64-apple-darwin)
git log -1 --format="ffmpeg source %h %cs $branch" > "$prefix/source.txt"
cp COPYING.LGPLv2.1 "$prefix/LICENSE.txt"
ls "$prefix/lib" | grep -c dylib
