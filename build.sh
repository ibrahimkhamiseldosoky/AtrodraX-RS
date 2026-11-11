#!/usr/bin/env bash
# Build the AtrodraX Rust kernel and produce a bootable GRUB ISO.
set -euo pipefail
cd "$(dirname "$0")"

# rustup installs cargo/rustc into ~/.cargo/bin, which is not always on PATH
# for non-login shells (for example when invoked from Windows).
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
fi

PROFILE="${PROFILE:-release}"
TARGET="x86_64-unknown-none"
OUT="target/${TARGET}/${PROFILE}/atrodrax"

echo "[build] cargo build --${PROFILE}"
cargo build --profile "${PROFILE}"

echo "[build] staging ISO tree"
rm -rf build/iso
mkdir -p build/iso/boot/grub
cp "${OUT}" build/iso/boot/kernel.bin
cp boot/grub/grub.cfg build/iso/boot/grub/grub.cfg

if command -v grub-mkrescue >/dev/null 2>&1; then
    echo "[build] grub-mkrescue"
    grub-mkrescue -o build/atrodrax.iso build/iso >/dev/null
    echo "[build] ISO: build/atrodrax.iso ($(stat -c %s build/atrodrax.iso) bytes)"
else
    echo "[build] grub-mkrescue not found; kernel staged at build/iso/boot/kernel.bin"
fi
