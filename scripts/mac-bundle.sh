#!/bin/sh
# on the mac, in the tree scripts/package-mac.sh sends: builds eyepiece.app for apple silicon and macos 13 and later,
# then signs it, has apple notarize it, and puts it in a disk image that is signed, notarized and stapled too.
#   sh scripts/mac-bundle.sh build                                   prints APP=<the unsigned app>
#   sh scripts/mac-bundle.sh sign <app> <identity> <notary profile>    in the login session (see `session`); prints DMG=<file>
#   sh scripts/mac-bundle.sh session <app> <identity> <notary profile> runs sign as a launchd job in gui/<uid> and waits
# an ssh session's keychains are locked, so signing runs in the login session, which needs the screen unlocked. the
# ffmpeg is the one spike/mac/setup.sh built in ~/src/eyepiece/spike/third_party/ffmpeg. everything is made in a new
# folder under /private/tmp; nothing is deleted.
set -eu
cd "$(dirname "$0")/.."
ff=$HOME/src/eyepiece/spike/third_party/ffmpeg
fail() { echo "FAILED: $1"; exit 1; }
# a command with its output kept in a file, shown only when it fails
run() {
  name=$1
  shift
  "$@" > "$work/$name.out" 2>&1 || { tail -8 "$work/$name.out"; fail "$name"; }
}
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)

case $1 in
build)
  work=/private/tmp/eyepiece-package-$(date +%Y%m%d-%H%M%S)
  app="$work/eyepiece.app"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks" "$app/Contents/Resources"
  free=$(df -g / | tail -1 | awk '{print $4}')
  [ "$free" -ge 4 ] || fail "only $free GB free on the mac"
  # without a deployment target the program asks for this mac's own macos; the ffmpeg was built for 13 too
  export FFMPEG_DIR=$ff LIBCLANG_PATH=/Library/Developer/CommandLineTools/usr/lib MACOSX_DEPLOYMENT_TARGET=13.0 CARGO_INCREMENTAL=0
  # niced and on half the cores: this mac is somebody's desk machine
  run cargo nice -n 10 cargo build --release -j4
  cp target/release/eyepiece "$app/Contents/MacOS/eyepiece"
  bin="$app/Contents/MacOS/eyepiece"
  # the ffmpeg libraries go inside the app, and every reference to them becomes relative to it
  for lib in libavcodec.63 libavformat.63 libavutil.61 libswresample.7; do
    cp "$ff/lib/$lib.dylib" "$app/Contents/Frameworks/" || fail "no $lib in $ff"
  done
  run rpath install_name_tool -add_rpath @executable_path/../Frameworks "$bin"
  for target in "$bin" "$app"/Contents/Frameworks/*.dylib; do
    for dep in $(otool -L "$target" | awk '{print $1}' | grep "^$ff/"); do
      install_name_tool -change "$dep" "@rpath/$(basename "$dep")" "$target" || fail "rewrite $dep"
    done
  done
  for lib in "$app"/Contents/Frameworks/*.dylib; do
    install_name_tool -id "@rpath/$(basename "$lib")" "$lib" || fail "id of $lib"
  done
  otool -L "$bin" "$app"/Contents/Frameworks/*.dylib | grep -q "$ff" && fail "a reference to $ff is left"
  # the texts windows ships beside eyepiece.exe, where a mac app keeps them
  r="$app/Contents/Resources"
  cp dist/mac/THIRD-PARTY-NOTICES.txt "$r/" || fail "no dist/mac/THIRD-PARTY-NOTICES.txt: scripts/package-mac.sh makes it"
  cp "$ff/LICENSE.txt" "$r/ffmpeg-LICENSE.txt" || fail "no ffmpeg license in $ff: spike/mac/setup.sh copies it"
  cp assets/fonts/IBM-Plex-LICENSE.txt "$r/fonts-IBM-Plex-LICENSE.txt"
  cp assets/fonts/Archivo-OFL.txt "$r/fonts-Archivo-OFL.txt"
  # the icon, from the windows icon's 512 picture
  set=$work/eyepiece.iconset
  mkdir -p "$set"
  for size in 16 32 128 256 512; do
    run "icon$size" sips -z $size $size assets/icon/eyepiece.png --out "$set/icon_${size}x${size}.png"
    [ $size = 512 ] || run "icon$size@2x" sips -z $((size * 2)) $((size * 2)) assets/icon/eyepiece.png --out "$set/icon_${size}x${size}@2x.png"
  done
  run iconutil iconutil -c icns "$set" -o "$r/eyepiece.icns"
  # finder's open with lists it for the videos it plays (src/browse's clips, less mxf and avi, which do not play yet),
  # an alternate that never takes the default from quicktime: get info > open with > change all does that. macos 13
  # may not declare mkv and webm itself, so they are imported, which gives way to a declaration of the system's
  cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>io.tridant.eyepiece</string>
  <key>CFBundleName</key><string>eyepiece</string>
  <key>CFBundleDisplayName</key><string>eyepiece</string>
  <key>CFBundleExecutable</key><string>eyepiece</string>
  <key>CFBundleIconFile</key><string>eyepiece</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.video</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>Copyright (c) 2026 Tridant, Inc.</string>
  <key>CFBundleDocumentTypes</key><array><dict>
    <key>CFBundleTypeName</key><string>video</string>
    <key>CFBundleTypeRole</key><string>Viewer</string>
    <key>LSHandlerRank</key><string>Alternate</string>
    <key>LSItemContentTypes</key><array>
      <string>com.apple.quicktime-movie</string>
      <string>public.mpeg-4</string>
      <string>com.apple.m4v-video</string>
      <string>org.matroska.mkv</string>
      <string>org.webmproject.webm</string>
    </array>
  </dict></array>
  <key>UTImportedTypeDeclarations</key><array>
    <dict>
      <key>UTTypeIdentifier</key><string>org.matroska.mkv</string>
      <key>UTTypeDescription</key><string>Matroska video</string>
      <key>UTTypeConformsTo</key><array><string>public.movie</string></array>
      <key>UTTypeTagSpecification</key><dict>
        <key>public.filename-extension</key><array><string>mkv</string></array>
        <key>public.mime-type</key><array><string>video/x-matroska</string></array>
      </dict>
    </dict>
    <dict>
      <key>UTTypeIdentifier</key><string>org.webmproject.webm</string>
      <key>UTTypeDescription</key><string>WebM video</string>
      <key>UTTypeConformsTo</key><array><string>public.movie</string></array>
      <key>UTTypeTagSpecification</key><dict>
        <key>public.filename-extension</key><array><string>webm</string></array>
        <key>public.mime-type</key><array><string>video/webm</string></array>
      </dict>
    </dict>
  </array>
</dict></plist>
EOF
  run plist plutil -lint "$app/Contents/Info.plist"
  echo "APP=$app"
  ;;

sign)
  app=$2 identity=$3 profile=$4
  work=$(dirname "$app")
  # hardened runtime and a timestamp, the libraries first: a seal covers what is inside it
  for item in "$app"/Contents/Frameworks/*.dylib "$app"; do
    run "sign-$(basename "$item")" codesign --force --options runtime --timestamp --sign "$identity" "$item"
  done
  run verify codesign --verify --deep --strict --verbose=2 "$app"
  run zip ditto -c -k --keepParent "$app" "$work/app.zip"
  run notarize-app xcrun notarytool submit "$work/app.zip" --keychain-profile "$profile" --wait
  grep -q "status: Accepted" "$work/notarize-app.out" || { cat "$work/notarize-app.out"; fail "apple did not accept the app"; }
  run staple-app xcrun stapler staple "$app"
  # the disk image: the app and a way to applications, signed, notarized and stapled itself
  mkdir -p "$work/image"
  cp -R "$app" "$work/image/"
  ln -s /Applications "$work/image/Applications"
  dmg="$work/eyepiece-$version-macos.dmg"
  run image hdiutil create -volname eyepiece -srcfolder "$work/image" -format UDZO "$dmg"
  run sign-image codesign --force --timestamp --sign "$identity" "$dmg"
  run notarize-image xcrun notarytool submit "$dmg" --keychain-profile "$profile" --wait
  grep -q "status: Accepted" "$work/notarize-image.out" || { cat "$work/notarize-image.out"; fail "apple did not accept the disk image"; }
  run staple-image xcrun stapler staple "$dmg"
  # gatekeeper's verdict on both, as a download would meet them
  spctl -a -vv -t exec "$app" 2>&1 | tail -2
  spctl -a -vv -t open --context context:primary-signature "$dmg" 2>&1 | tail -2
  spctl -a -t exec "$app" && spctl -a -t open --context context:primary-signature "$dmg" || fail "gatekeeper refuses it"
  echo "DMG=$dmg"
  ;;

session)
  # the login session's keychain is open; ssh's is not. the job file sits beside the work, not in LaunchAgents, so it
  # never runs again at login, and it is taken out of launchd when it ends
  [ "$(ioreg -n Root -d1 -a | grep -c CGSSessionScreenIsLocked)" = 0 ] || fail "the mac's screen is locked: unlock it first"
  app=$2
  work=$(dirname "$app")
  label=io.tridant.eyepiece.package
  log="$work/sign.log"
  printf "cd '%s'\nsh scripts/mac-bundle.sh sign '%s' '%s' '%s'\necho \"JOB-EXIT=\$?\"\n" "$(pwd)" "$app" "$3" "$4" > "$work/sign.sh"
  cat > "$work/$label.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>$label</string>
  <key>ProgramArguments</key><array><string>/bin/zsh</string><string>-l</string><string>$work/sign.sh</string></array>
  <key>StandardOutPath</key><string>$log</string>
  <key>StandardErrorPath</key><string>$log</string>
  <key>RunAtLoad</key><true/>
</dict></plist>
EOF
  : > "$log"
  launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
  launchctl bootstrap "gui/$(id -u)" "$work/$label.plist"
  # notarizing twice takes minutes; half an hour means it is stuck
  n=0
  until grep -q '^JOB-EXIT=' "$log"; do
    n=$((n + 1))
    [ $n -le 1800 ] || { cat "$log"; fail "no result after 30 minutes; the job is still loaded as $label"; }
    sleep 1
  done
  launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
  grep -v '^JOB-EXIT=' "$log" || true
  exit "$(sed -n 's/^JOB-EXIT=//p' "$log" | tail -1)"
  ;;
*)
  fail "build, sign or session"
  ;;
esac
