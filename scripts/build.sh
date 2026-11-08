#!/usr/bin/env bash
# Build in the Linux filesystem (ext4) and copy only the binary to Windows.
# Working directly on /mnt/d is a 9p mount and makes cargo several times slower.
set -euo pipefail

SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DST="$HOME/valtrack-build"

mkdir -p "$DST"
if command -v rsync >/dev/null 2>&1; then
  rsync -a --delete --exclude target "$SRC"/ "$DST"/
else
  rm -rf "$DST/src" "$DST/.cargo"
  mkdir -p "$DST"
  cp -a "$SRC"/Cargo.toml "$SRC"/Cargo.lock "$DST"/ 2>/dev/null || true
  cp -a "$SRC"/src "$DST"/
  cp -a "$SRC"/.cargo "$DST"/
fi

cd "$DST"
. "$HOME/.cargo/env"
cargo build --release --target x86_64-pc-windows-gnu

mkdir -p "$SRC/dist"
cp target/x86_64-pc-windows-gnu/release/valtrack.exe "$SRC/dist/valtrack.exe"
ls -lh "$SRC/dist/valtrack.exe"
