#!/usr/bin/env bash
# Boot the AtrodraX Rust ISO in QEMU.
#
# Usage:
#   ./run_qemu.sh                 # graphical window
#   ./run_qemu.sh --headless      # no display, serial log on stdout
#
# Environment:
#   QEMU=...      QEMU binary (default: qemu-system-x86_64)
#   MEMORY=...    guest RAM (default: 512M)
#   DISK=...      raw FAT32 image (default: ./disk.img; skipped if absent)
#   ISO=...       ISO path (default: build/atrodrax.iso)
set -euo pipefail
cd "$(dirname "$0")"

ISO="${ISO:-build/atrodrax.iso}"
DISK="${DISK:-disk.img}"
MEMORY="${MEMORY:-512M}"
QEMU="${QEMU:-qemu-system-x86_64}"
HEADLESS=0

for arg in "$@"; do
    case "$arg" in
        --headless|-n) HEADLESS=1 ;;
        --help|-h)
            sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

if [ ! -f "$ISO" ]; then
    echo "error: $ISO not found; run ./build.sh first" >&2
    exit 1
fi

ARGS=(
    -cdrom "$ISO"
    -m "$MEMORY"
    -no-reboot
    -boot order=d
    -nic user,model=e1000
)

if [ -f "$DISK" ]; then
    ARGS+=(
        -drive "file=$DISK,format=raw,if=none,id=disk0"
        -device virtio-blk-pci,drive=disk0,disable-legacy=off,disable-modern=on
    )
else
    echo "note: $DISK not found; booting without a persistent disk" >&2
fi

if [ "$HEADLESS" = 1 ]; then
    ARGS+=(-display none -serial mon:stdio)
else
    ARGS+=(-serial mon:stdio)
fi

echo "booting $ISO with $QEMU (memory $MEMORY, headless=$HEADLESS)"
exec "$QEMU" "${ARGS[@]}"
