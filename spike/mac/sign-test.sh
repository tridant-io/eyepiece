#!/bin/sh
# signing gate on the mac: can an app that carries its own ffmpeg libraries be signed, notarized, stapled and run?
#   sh spike/mac/sign-test.sh "<developer id application identity>" <notarytool keychain profile> [keychain file]
# builds a throwaway .app around gate5_share with the ffmpeg dylibs inside it, the way the player will ship.
# must run in the login session (a launchd job in gui/<uid>): keychains are locked in an ssh session.
# ends with SIGN-EXIT=<code>. everything is written under /private/tmp/eyepiece-signtest-<time>.
set -u
identity=$1
profile=$2
keychain=${3:-}
here=$(cd "$(dirname "$0")" && pwd)
ff="$here/../third_party/ffmpeg/lib"
work=/private/tmp/eyepiece-signtest-$(date +%Y%m%d-%H%M%S)
app="$work/EyepieceSigningTest.app"
bin="$app/Contents/MacOS/eyepiece-signing-test"
fw="$app/Contents/Frameworks"
step() { echo "--- $1"; }
fail() { echo "FAILED at: $1"; echo "SIGN-EXIT=1"; exit 1; }
# runs a command with its output kept in a file; stops the gate if it fails
run() {
  name=$1
  shift
  "$@" > "$work/$name.out" 2>&1 || { tail -5 "$work/$name.out"; fail "$name"; }
}

step "bundle"
mkdir -p "$app/Contents/MacOS" "$fw" || fail bundle
cp "$here/target/release/gate5_share" "$bin" || fail "copy the program (build it first: sh spike/mac/gate.sh gate5_share)"
cat > "$app/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>io.tridant.eyepiece.signing-test</string>
  <key>CFBundleName</key><string>EyepieceSigningTest</string>
  <key>CFBundleExecutable</key><string>eyepiece-signing-test</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.0.0</string>
  <key>CFBundleVersion</key><string>0</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
</dict></plist>
EOF
# the ffmpeg libraries move inside the bundle and every reference to them becomes relative to it
for lib in "$ff"/lib*.[0-9]*.dylib; do
  case "$lib" in *.*.*.*.dylib) continue ;; esac
  cp "$lib" "$fw/" || fail "copy $lib"
done
install_name_tool -add_rpath @executable_path/../Frameworks "$bin" || fail rpath
for target in "$bin" "$fw"/*.dylib; do
  for dep in $(otool -L "$target" | awk '{print $1}' | grep "^$ff/"); do
    install_name_tool -change "$dep" "@rpath/$(basename "$dep")" "$target" || fail "rewrite $dep"
  done
done
for lib in "$fw"/*.dylib; do
  install_name_tool -id "@rpath/$(basename "$lib")" "$lib" || fail "id of $lib"
done
ls "$fw" | tr '\n' ' '
echo

step "sign (hardened runtime, timestamp), libraries first"
n=0
for item in "$fw"/*.dylib "$app"; do
  n=$((n + 1))
  run "sign$n" codesign --force --options runtime --timestamp ${keychain:+--keychain "$keychain"} --sign "$identity" "$item"
done
run verify codesign --verify --deep --strict --verbose=2 "$app"
tail -2 "$work/verify.out"
codesign -dv "$app" 2>&1 | grep -E "^Authority|^TeamIdentifier|^Timestamp|flags=" | head -6

step "run the signed app: its libraries must load under the hardened runtime"
run launch "$bin"
tail -1 "$work/launch.out"

step "notarize"
run zip ditto -c -k --keepParent "$app" "$work/app.zip"
run notarize xcrun notarytool submit "$work/app.zip" --keychain-profile "$profile" --wait
grep -E "^ *(id|status):" "$work/notarize.out" | sort -u
grep -q "status: Accepted" "$work/notarize.out" || fail "the notary service did not accept it"
run staple xcrun stapler staple "$app"
tail -1 "$work/staple.out"

step "gatekeeper's verdict"
spctl -a -vv -t exec "$app" 2>&1 | sed -E 's/^.*EyepieceSigningTest.app: //'
spctl -a -t exec "$app" 2>/dev/null
echo "SIGN-EXIT=$?"
