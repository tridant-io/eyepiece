#!/bin/sh
# on the mac: runs one gate program against the ffmpeg that setup.sh built.
#   sh spike/mac/gate.sh gate5_share spike/media/tiny_hap.mov
set -eu
here=$(cd "$(dirname "$0")" && pwd)
export FFMPEG_DIR="$here/../third_party/ffmpeg"
export LIBCLANG_PATH=/Library/Developer/CommandLineTools/usr/lib
gate=$1
shift
exec cargo run --quiet --manifest-path "$here/Cargo.toml" --release --bin "$gate" -- "$@"
