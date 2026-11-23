#!/bin/sh
# the scratch smb share for gate 6. runs inside the eyepiece-gate6 wsl distro; gate6.ps1 calls it.
set -eu
case "$1" in
setup) # $2 = the share password. safe to run again.
  command -v smbd >/dev/null || apk add --no-cache samba >/dev/null
  id gate6 >/dev/null 2>&1 || adduser -D -H -s /sbin/nologin gate6
  printf '%s\n%s\n' "$2" "$2" | smbpasswd -s -a gate6 >/dev/null
  mkdir -p /srv/gate6 /var/log/samba
  # the log holds two things: every smb2 request by name (smb2:10), and through the audit module the path of
  # every open a client asked for (create_file). its openat lines are the server's own work, not requests.
  cat > /etc/samba/smb.conf <<'EOF'
[global]
  server role = standalone server
  disable netbios = yes
  smb ports = 445
  map to guest = never
  log file = /var/log/samba/smbd.log
  max log size = 0
  log level = 1 smb2:10
  vfs objects = full_audit
  full_audit:prefix = %u|%S
  full_audit:success = create_file openat
  full_audit:failure = none
  full_audit:syslog = false
[gate6]
  path = /srv/gate6
  read only = yes
  valid users = gate6
EOF
  pgrep smbd >/dev/null || smbd -D
  smbd --version
  ;;
fresh) # $2 = run name. one folder per mode, same contents, never seen by the windows client before.
  for m in open list stat attrs; do
    d="/srv/gate6/$2-$m"
    mkdir "$d"
    i=1
    while [ $i -le 20 ]; do
      head -c 4096 /dev/urandom > "$d/clip_$(printf %02d $i).mov"
      i=$((i + 1))
    done
    chown -R gate6 "$d"
  done
  ;;
times) # $2 = a folder in the share. three hashes on one line: every file's access, modify and change time.
  for t in X Y Z; do
    find "/srv/gate6/$2" -type f -exec stat -c "%n %$t" {} + | sort | md5sum | cut -c1-8
  done | tr '\n' ' '
  ;;
esac
