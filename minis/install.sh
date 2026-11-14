#!/bin/sh
# Installs ssboot into Open Minis and runs it, which pairs this iPhone with
# itself (Open Minis 1.16 and later; with an older Minis, the pairing comes from
# ssboot-pair in rootshell), signs SideStore with your Apple Account and
# installs it.
#
# In Minis's terminal (Minis › Settings › Terminal):
#
#   wget -qO- https://raw.githubusercontent.com/FrizzleM/siboot/ssboot/minis/install.sh | sh
#
# Afterwards `ssboot` runs it again, without downloading.
set -eu

BASE="${SSBOOT_BASE:-https://raw.githubusercontent.com/FrizzleM/siboot/ssboot/minis}"
# Of bin/ssboot-aarch64, updated with it: a download that doesn't match (an
# error page, a cut-off transfer) is never run. SIZE only draws the progress
# bar.
SHA256="d3d0072879a9a703623a35a3c679c74d8f024e0b3f8f1d84ea2d6880f907c8f4"
SIZE=12209016

DIR="$HOME/.ssboot/bin"
BIN="$DIR/ssboot"

# On a terminal, one line that redraws itself, as ssboot's own screen does.
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
  *) fail "ssboot runs in Open Minis on an iPhone (aarch64); this is $(uname -m)." ;;
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
  # "  ⠋ Downloading ssboot " and " 42%", a column spare.
  cells=$((COLS - 29))
  if [ "$cells" -ge $((12 + ${#size})) ]; then
    cells=$((cells - ${#size}))
  else
    size=
  fi
  [ "$cells" -gt 24 ] && cells=24
  [ "$cells" -lt 4 ] && cells=4
  full=$((percent * cells / 100))
  printf '\r\033[2K  \033[36m%s\033[0m Downloading ssboot \033[36m%s\033[0m\033[2m%s\033[0m %3d%%%s' \
    "$spin" "$(repeat "$full" ━)" "$(repeat $((cells - full)) ─)" "$percent" "$size"
}

# The download in the background, and a frame every tenth of a second from
# how much of it has arrived.
download() {
  rm -f "$BIN.part"
  if [ -z "$FANCY" ]; then
    echo "downloading ssboot ($(megabytes "$SIZE") MB) …"
    fetch "$BASE/bin/ssboot-aarch64" "$BIN.part"
    return
  fi
  fetch "$BASE/bin/ssboot-aarch64" "$BIN.part" < /dev/null 2> /dev/null &
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
  ok "ssboot is up to date"
else
  if ! download; then
    rm -f "$BIN.part"
    fail "Couldn't download ssboot. Check the internet connection and try again."
  fi
  if [ "$(sha "$BIN.part")" != "$SHA256" ]; then
    rm -f "$BIN.part"
    fail "The ssboot download didn't match its checksum. Try again in a few minutes."
  fi
  chmod 755 "$BIN.part"
  mv "$BIN.part" "$BIN"
  ok "ssboot downloaded and checked ($(megabytes "$SIZE") MB)"
fi

# So `ssboot` works on its own from now on (Minis's shell runs as root).
ln -sf "$BIN" /usr/local/bin/ssboot 2> /dev/null || true

# Piped into sh, stdin is this script, and ssboot's prompts need the terminal.
# The test opens it in a subshell: a redirection that fails on `:`, a special
# built-in, would end this shell, where there's no terminal at all.
if [ -t 0 ]; then
  exec "$BIN" "$@"
elif [ -r /dev/tty ] && (: < /dev/tty) 2> /dev/null; then
  exec "$BIN" "$@" < /dev/tty
else
  exec "$BIN" "$@"
fi
