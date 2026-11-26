#!/bin/sh
# from the windows dev machine, in git bash: makes the mac release, dist/eyepiece-<version>-macos.dmg, on a mac.
#   sh scripts/package-mac.sh user@host <notarytool keychain profile> [signing identity]
# the notices are written here (scripts/notices.ps1 -Target aarch64-apple-darwin, from the mac's ffmpeg), the tree is
# sent to the mac's ~/src/eyepiece-player as scripts/mac-test.sh sends it, and scripts/mac-bundle.sh builds, signs and
# notarizes there. the mac's screen must be unlocked for signing. the identity defaults to the one "Developer ID
# Application" certificate in the mac's keychain.
set -eu
host=$1
profile=$2
identity=${3:-Developer ID Application}
cd "$(dirname "$0")/.."
# up to the closing quote: a checkout with windows line ends has a \r after it
version=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' Cargo.toml | head -1)
out=dist/eyepiece-$version-macos.dmg
mkdir -p dist/mac/ffmpeg
mac() { ssh -o BatchMode=yes "$host" zsh -lc "'$1'"; }

echo "[package-mac] the mac's ffmpeg, for the notices"
mac 'cat ~/src/eyepiece/spike/third_party/ffmpeg/lib/libavutil.61.dylib' > dist/mac/ffmpeg/libavutil.61.dylib
mac 'cat ~/src/eyepiece/spike/third_party/ffmpeg/source.txt' > dist/mac/ffmpeg/source.txt
cargo fetch --quiet --target aarch64-apple-darwin
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/notices.ps1 -Target aarch64-apple-darwin -Ffmpeg dist/mac/ffmpeg -Out dist/mac/THIRD-PARTY-NOTICES.txt

echo "[package-mac] sending the tree"
tar -cf - Cargo.toml Cargo.lock build.rs .cargo assets src scripts/mac-bundle.sh dist/mac/THIRD-PARTY-NOTICES.txt |
  ssh -o BatchMode=yes "$host" 'mkdir -p ~/src/eyepiece-player && cd ~/src/eyepiece-player && tar -xf -'

echo "[package-mac] building $version"
app=$(mac 'cd ~/src/eyepiece-player && sh scripts/mac-bundle.sh build' | tee /dev/stderr | sed -n 's/^APP=//p')
[ -n "$app" ] || { echo "[package-mac] the build failed"; exit 1; }

echo "[package-mac] signing and notarizing in the mac's login session"
dmg=$(mac "cd ~/src/eyepiece-player && sh scripts/mac-bundle.sh session \"$app\" \"$identity\" \"$profile\"" | tee /dev/stderr | sed -n 's/^DMG=//p')
[ -n "$dmg" ] || { echo "[package-mac] signing or notarizing failed"; exit 1; }

mac "cat \"$dmg\"" > "$out"
there=$(mac "shasum -a 256 \"$dmg\"" | cut -d' ' -f1)
here=$(sha256sum "$out" | cut -d' ' -f1)
[ "$there" = "$here" ] || { echo "[package-mac] $out did not arrive whole"; exit 1; }
echo "[package-mac] $out ($(($(wc -c < "$out") / 1048576)) MB), sha-256 $here"
