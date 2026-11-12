#!/bin/sh
# Installs siboot into Open Minis and runs it, which takes the pairing from
# siboot-pair in rootshell, signs SideInstaller with your Apple Account and
# installs it.
#
# In Minis's terminal (Minis › Settings › Terminal), with siboot-pair still
# running in rootshell:
#
#   wget -qO- https://raw.githubusercontent.com/FrizzleM/siboot/main/minis/install.sh | sh
#
# Afterwards `siboot` runs it again, without downloading.
set -eu

BASE="${SIBOOT_BASE:-https://raw.githubusercontent.com/FrizzleM/siboot/main/minis}"
# Of bin/siboot-aarch64, updated with it: a download that doesn't match (an
# error page, a cut-off transfer) is never run. SIZE only draws the progress
# bar.
SHA256="f643902d57176a1be6abbb67e46cc3e298cc3e372b48c0ee1542459876421f2a"
SIZE=11422096

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

fail() {
  say ✗ '1;31' "$1" >&2
  exit 1
}

case "$(uname -m)" in
  aarch64 | arm64) ;;
  *) fail "siboot runs in Open Minis on an iPhone (aarch64); this is $(uname -m)." ;;
esac

fetch() {
  if command -v curl > /dev/null 2>&1; then
    curl -fsSL "$1" -o "$2"
  else
    wget -qO "$2" "$1"
  fi
}

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

# The download in the background, and a frame every tenth of a second from
# how much of it has arrived.
download() {
  rm -f "$BIN.part"
  if [ -z "$FANCY" ]; then
    echo "downloading siboot ($(megabytes "$SIZE") MB) …"
    fetch "$BASE/bin/siboot-aarch64" "$BIN.part"
    return
  fi
  fetch "$BASE/bin/siboot-aarch64" "$BIN.part" < /dev/null 2> /dev/null &
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
  if ! download; then
    rm -f "$BIN.part"
    fail "Couldn't download siboot. Check the internet connection and try again."
  fi
  if [ "$(sha "$BIN.part")" != "$SHA256" ]; then
    rm -f "$BIN.part"
    fail "The siboot download didn't match its checksum. Try again in a few minutes."
  fi
  chmod 755 "$BIN.part"
  mv "$BIN.part" "$BIN"
  ok "siboot downloaded and checked ($(megabytes "$SIZE") MB)"
fi

# So `siboot` works on its own from now on (Minis's shell runs as root).
ln -sf "$BIN" /usr/local/bin/siboot 2> /dev/null || true

# Piped into sh, stdin is this script, and siboot's prompts need the terminal.
if [ -t 0 ]; then
  exec "$BIN" "$@"
elif [ -r /dev/tty ] && { : < /dev/tty; } 2> /dev/null; then
  exec "$BIN" "$@" < /dev/tty
else
  exec "$BIN" "$@"
fi
