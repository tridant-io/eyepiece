#!/bin/sh
# on the mac: runs a command inside the login session and prints its output.
#   sh spike/mac/session-run.sh <name> <command> [args...]
# an ssh session has no window server and its keychains are locked; a launchd job in gui/<uid> has both.
# the job file is written under <repo>/jobs, not ~/Library/LaunchAgents, so it never runs again at login,
# and the job is removed from launchd when the command ends. refuses to run while the screen is locked.
set -eu
name=$1
shift
dir=$(cd "$(dirname "$0")/../.." && pwd)/jobs
label=io.tridant.eyepiece.$name
log="$dir/$name.log"
mkdir -p "$dir"
if [ "$(ioreg -n Root -d1 -a | grep -c CGSSessionScreenIsLocked)" != 0 ]; then
  echo "the mac's screen is locked: unlock it first"
  exit 1
fi
# the command goes into a small script, each argument quoted, so the job file needs no escaping
{
  printf 'cd %s\n' "'$(dirname "$dir")'"
  for a in "$@"; do printf "'%s' " "$(printf %s "$a" | sed "s/'/'\\\\''/g")"; done
  printf '\necho "JOB-EXIT=$?"\n'
} > "$dir/$name.sh"
cat > "$dir/$label.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>$label</string>
  <key>ProgramArguments</key><array><string>/bin/sh</string><string>$dir/$name.sh</string></array>
  <key>StandardOutPath</key><string>$log</string>
  <key>StandardErrorPath</key><string>$log</string>
  <key>RunAtLoad</key><true/>
</dict></plist>
EOF
: > "$log"
launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$dir/$label.plist"
n=0
until grep -q '^JOB-EXIT=' "$log"; do
  n=$((n + 1))
  if [ $n -gt 900 ]; then
    echo "no result after 15 minutes; the job is still loaded as $label"
    cat "$log"
    exit 1
  fi
  sleep 1
done
launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
code=$(sed -n 's/^JOB-EXIT=//p' "$log" | tail -1)
grep -v '^JOB-EXIT=' "$log" || true
exit "$code"
