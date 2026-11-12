#!/usr/bin/env bash
# Build the Linux-ABI test programs and drop them into the kernel's embedded
# RAMFS assets. The kernel only runs *static* Linux binaries: there is no ELF
# interpreter loader yet, so keep -static and avoid PT_INTERP.
set -euo pipefail
cd "$(dirname "$0")"

OUT="../../kernel/assets/user"
mkdir -p "$OUT"

CC="${CC:-gcc}"
CFLAGS=(
    -static -nostdlib -no-pie
    -mgeneral-regs-only -fno-stack-protector
    -fno-asynchronous-unwind-tables
    -O2
)

"$CC" "${CFLAGS[@]}" -o "$OUT/hello_linux.elf" hello.c
echo "[userland] hello_linux.elf -> $OUT"
