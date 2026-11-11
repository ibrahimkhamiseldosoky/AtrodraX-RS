# AtrodraX (Rust edition) — build and run helpers.
#
#   make            build the kernel and GRUB ISO
#   make run        boot the ISO in QEMU (graphical window)
#   make run-headless  boot with no display, serial log on stdout
#   make clean      remove build products
#
# The heavy lifting lives in build.sh so it can also be invoked directly.

SHELL := /bin/bash
PROFILE ?= release
ISO := build/atrodrax.iso
DISK ?= ../../disk.img
QEMU ?= qemu-system-x86_64
MEMORY ?= 512M

.PHONY: all iso build run run-headless clean check

all: iso

iso: build

build:
	@bash build.sh

run: iso
	@DISK="$(abspath $(DISK))" MEMORY="$(MEMORY)" QEMU="$(QEMU)" bash run_qemu.sh

run-headless: iso
	@DISK="$(abspath $(DISK))" MEMORY="$(MEMORY)" QEMU="$(QEMU)" \
		bash run_qemu.sh --headless

clean:
	cargo clean
	rm -rf build/iso
	rm -f "$(ISO)"

check:
	cargo check --profile "$(PROFILE)"
