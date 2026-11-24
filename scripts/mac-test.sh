#!/bin/sh
# from the windows dev machine, in git bash: sends the working tree to a mac and runs the unit tests there.
#   sh scripts/mac-test.sh user@host [arguments for cargo test]
# the mac keeps its copy in ~/src/eyepiece-player, and needs rust, apple's command line tools and the ffmpeg that
# spike/mac/setup.sh built (in ~/src/eyepiece/spike/third_party/ffmpeg). nothing there is deleted by this script.
# the build is niced and on half the cores: that mac is somebody's desk machine. it works over ssh with the screen
# locked: the tests draw into pictures with no window. it refuses to build with under 4 GB free on the mac's disk.
set -eu
host=$1
shift
cd "$(dirname "$0")/.."
tar -cf - Cargo.toml Cargo.lock build.rs .cargo assets src tests/media | ssh -o BatchMode=yes "$host" 'mkdir -p ~/src/eyepiece-player && cd ~/src/eyepiece-player && tar -xf -'
ssh -o BatchMode=yes "$host" zsh -lc "'
  cd ~/src/eyepiece-player
  free=\$(df -g / | tail -1 | awk \"{print \\\$4}\")
  if [ \"\$free\" -lt 4 ]; then echo \"only \$free GB free on the mac: not building\"; exit 1; fi
  export FFMPEG_DIR=\$HOME/src/eyepiece/spike/third_party/ffmpeg LIBCLANG_PATH=/Library/Developer/CommandLineTools/usr/lib
  export DYLD_LIBRARY_PATH=\$FFMPEG_DIR/lib CARGO_INCREMENTAL=0
  nice -n 10 cargo test -j4 $* 2>&1 | grep -E \"^error|^test .*FAILED|^test result|panicked\" -A 12
  echo \"load \$(sysctl -n vm.loadavg) · \$(df -g / | tail -1 | awk \"{print \\\$4}\") GB free\"
'"
