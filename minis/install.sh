#!/bin/sh
# Installs siboot into Open Minis and runs it, which pairs this iPhone with
# itself (Open Minis 1.16 and later; siboot stops in an older one), signs
# SideInstaller with your Apple Account and installs it.
#
# In Minis's terminal (Minis › Settings › Terminal):
#
#   wget -qO- https://raw.githubusercontent.com/FrizzleM/siboot/main/minis/install.sh | sh
#
# Where raw.githubusercontent.com is blocked (mainland China, Iran), the same
# script through jsDelivr:
#
#   wget -qO- https://fastly.jsdelivr.net/gh/FrizzleM/siboot@main/minis/install.sh | sh
#
# Afterwards `siboot` runs it again, without downloading.
set -eu

# Where bin/siboot-aarch64 comes from, tried in order: GitHub, then the same
# file through jsDelivr's CDNs, which can get through where GitHub's raw files
# don't. Whichever answers, the copy has to match SHA256, so a mirror can't
# change what runs. SIBOOT_BASE replaces the list (one URL, or several).
SOURCES="https://raw.githubusercontent.com/FrizzleM/siboot/main/minis
https://fastly.jsdelivr.net/gh/FrizzleM/siboot@main/minis
https://gcore.jsdelivr.net/gh/FrizzleM/siboot@main/minis
https://cdn.jsdelivr.net/gh/FrizzleM/siboot@main/minis"
SOURCES="${SIBOOT_BASE:-$SOURCES}"
# Of bin/siboot-aarch64, updated with it: a download that doesn't match (an
# error page, a cut-off transfer, a stale mirror) is never run. SIZE only
# draws the progress bar.
SHA256="05d0a2ef07f99caca762efc6a6e4a7a8b1a691186dd1158759bf18755093c760"
SIZE=12209056

DIR="$HOME/.siboot/bin"
BIN="$DIR/siboot"

# On a terminal, one line that redraws itself, as siboot's own screen does.
FANCY=
if [ -t 1 ] && [ "${TERM:-}" != dumb ] && [ -z "${NO_COLOR:-}" ]; then
  FANCY=1
fi

# The terminal's width, so no line wraps by itself.
COLS=$({ stty size < /dev/tty; } 2> /dev/null | cut -d ' ' -f 2) || COLS=
case "$COLS" in
  '' | *[!0-9]*) COLS=40 ;;
esac

# A line that stays: a mark in a color, then the text wrapped under itself.
say() {
  if [ -z "$FANCY" ]; then
    echo "$3"
    return
  fi
  printf '\r\033[2K'
  first=1
  printf '%s\n' "$3" | fold -s -w $((COLS - 5)) | while IFS= read -r line; do
    if [ -n "$first" ]; then
      printf '  \033[%sm%s\033[0m %s\n' "$2" "$1" "$line"
      first=
    else
      printf '    %s\n' "$line"
    fi
  done
}

ok() { say ✓ '1;32' "$1"; }
warn() { say ▲ '1;33' "$1"; }

fail() {
  say ✗ '1;31' "$1" >&2
  exit 1
}

case "$(uname -m)" in
  aarch64 | arm64) ;;
  *) fail "siboot runs in Open Minis on an iPhone (aarch64); this is $(uname -m)." ;;
esac

# Where a host is blocked, its connections can hang instead of failing:
# give up on one that sends nothing for 30 seconds (busybox wget's own limit
# is 15 minutes).
fetch() {
  if command -v curl > /dev/null 2>&1; then
    curl -fsSL --connect-timeout 20 -y 30 -Y 1 "$1" -o "$2"
  else
    wget -q -T 30 -O "$2" "$1"
  fi
}

# "raw.githubusercontent.com" from a URL.
host() { printf '%s\n' "$1" | cut -d / -f 3; }

sha() { sha256sum "$1" | cut -d ' ' -f 1; }

# `n` copies of a string.
repeat() {
  n=$1
  out=
  while [ "$n" -gt 0 ]; do
    out="$out$2"
    n=$((n - 1))
  done
  printf '%s' "$out"
}

# Tenths of a megabyte, as "10.7".
megabytes() {
  tenths=$(($1 * 10 / 1048576))
  printf '%d.%d' $((tenths / 10)) $((tenths % 10))
}

# One frame: spinner, bar, percentage and, where there's room, megabytes.
frame() {
  have=$1
  case $2 in
    0) spin=⠋ ;; 1) spin=⠙ ;; 2) spin=⠹ ;; 3) spin=⠸ ;; 4) spin=⠼ ;;
    5) spin=⠴ ;; 6) spin=⠦ ;; 7) spin=⠧ ;; 8) spin=⠇ ;; *) spin=⠏ ;;
  esac
  percent=$((have * 100 / SIZE))
  [ "$percent" -gt 99 ] && percent=99
  size=" $(megabytes "$have")/$(megabytes "$SIZE") MB"
  # "  ⠋ Downloading siboot " and " 42%", a column spare.
  cells=$((COLS - 29))
  if [ "$cells" -ge $((12 + ${#size})) ]; then
    cells=$((cells - ${#size}))
  else
    size=
  fi
  [ "$cells" -gt 24 ] && cells=24
  [ "$cells" -lt 4 ] && cells=4
  full=$((percent * cells / 100))
  printf '\r\033[2K  \033[36m%s\033[0m Downloading siboot \033[36m%s\033[0m\033[2m%s\033[0m %3d%%%s' \
    "$spin" "$(repeat "$full" ━)" "$(repeat $((cells - full)) ─)" "$percent" "$size"
}

# The download from one source in the background, and a frame every tenth
# of a second from how much of it has arrived.
download() {
  rm -f "$BIN.part"
  if [ -z "$FANCY" ]; then
    echo "downloading siboot ($(megabytes "$SIZE") MB) from $(host "$1") …"
    fetch "$1/bin/siboot-aarch64" "$BIN.part"
    return
  fi
  fetch "$1/bin/siboot-aarch64" "$BIN.part" < /dev/null 2> /dev/null &
  pid=$!
  i=0
  while kill -0 "$pid" 2> /dev/null; do
    have=$({ wc -c < "$BIN.part"; } 2> /dev/null || echo 0)
    frame $((have + 0)) "$i"
    i=$(((i + 1) % 10))
    sleep 0.1 2> /dev/null || sleep 1
  done
  wait "$pid"
}

mkdir -p "$DIR"
if [ -f "$BIN" ] && [ "$(sha "$BIN")" = "$SHA256" ]; then
  ok "siboot is up to date"
else
  got=
  why=
  mismatched=
  for base in $SOURCES; do
    if [ -n "$why" ]; then
      warn "$why Trying $(host "$base")."
    fi
    if download "$base"; then
      if [ "$(sha "$BIN.part")" = "$SHA256" ]; then
        got=$base
        break
      fi
      mismatched=1
      why="The copy from $(host "$base") didn't match its checksum."
    else
      why="Couldn't download siboot from $(host "$base")."
    fi
    rm -f "$BIN.part"
  done
  if [ -z "$got" ]; then
    rm -f "$BIN.part"
    if [ -n "$mismatched" ]; then
      fail "The siboot download didn't match its checksum: a new siboot may have just come out. Try again in a few minutes."
    fi
    fail "Couldn't download siboot from GitHub or its mirrors. Check the internet connection. Where GitHub is blocked, turn on a VPN that reaches it and run this again: siboot asks for LocalDevVPN once it has what it needs."
  fi
  chmod 755 "$BIN.part"
  mv "$BIN.part" "$BIN"
  # Which source, when it wasn't the first.
  case "$SOURCES" in
    "$got"*) from= ;;
    *) from=" from $(host "$got")" ;;
  esac
  ok "siboot downloaded$from and checked ($(megabytes "$SIZE") MB)"
fi

# So `siboot` works on its own from now on (Minis's shell runs as root).
ln -sf "$BIN" /usr/local/bin/siboot 2> /dev/null || true

# Piped into sh, stdin is this script, and siboot's prompts need the terminal.
# The test opens it in a subshell: a redirection that fails on `:`, a special
# built-in, would end this shell, where there's no terminal at all.
if [ -t 0 ]; then
  exec "$BIN" "$@"
elif [ -r /dev/tty ] && (: < /dev/tty) 2> /dev/null; then
  exec "$BIN" "$@" < /dev/tty
else
  exec "$BIN" "$@"
fi
