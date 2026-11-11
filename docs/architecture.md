# Architecture notes

This document describes how the Rust port boots and how its subsystems fit
together. Paths are relative to `kernel/src`.

## Boot sequence

1. GRUB loads the Multiboot2 kernel and jumps to `_start` in `entry.S`.
2. `entry.S` saves the Multiboot magic/info pointer, builds identity page
   tables (PML4 → PDPT → four page directories of 2 MiB pages covering the
   first 4 GiB), enables PAE/LME/NXE, loads CR3 and far-jumps into 64-bit
   `kernel_main`.
3. `main.rs` validates the Multiboot2 magic, then runs, in order:
   `boot::init`, GDT/TSS/IDT/PIC, PMM, heap, VMM, PIT, framebuffer, RTC,
   syscall gate, task/scheduler, ELF self-test, PS/2 keyboard and mouse, PCI,
   e1000, virtio-blk, VFS, RAMFS, FAT32, font/window/compositor, networking,
   boot self-tests, interrupts, `system::run`.
4. `boot::init` scans Multiboot2 tags for a memory map, framebuffer info and
   modules. If a memory-map tag is present it takes precedence over the basic
   meminfo tag so usable RAM is counted once.

## Memory

- `mem/pmm.rs`: bitmap over the first 4 GiB (one bit per 4 KiB page). Every
  page starts reserved; usable regions from the memory map are freed, then
  the kernel image, Multiboot info, framebuffer and modules are reserved.
  Allocation is a rotating first-fit over the bitmap.
- `mem/vmm.rs`: kernel page tables are built at boot. `alloc_frame` gives a
  zeroed page. `map_user_range` creates per-process PML4s that share the
  kernel PDPT/PD tree. When a user 4 KiB page falls inside a 2 MiB kernel
  huge page, the huge mapping is split into a page table first. PML4/PDPT
  entries are `PTE_USER` and pages writable.
- `mem/heap.rs`: a 64 MiB `.bss` arena with a first-fit free list and
  coalescing, behind a spin lock. `alloc_error_handler` halts with a panic
  message.

Address constants used by the loader:

| Range | Use |
|-------|-----|
| `0x0000_0000`–`0xFFFF_FFFF` | identity-mapped kernel / physical memory |
| `0x1000_0000`–`0x1001_0000` | per-task 64 KiB user stack |
| `0x2000_0000` | default program break (`brk`) |
| `0x5000_0000` | ELF load base |

## Interrupts and timers

`arch/idt.rs` installs 256 gates. IRQ0 (PIT, 100 Hz) drives `sync::ticks` and
the scheduler tick counter; IRQ1 delivers PS/2 keyboard scancodes; IRQ12
delivers mouse packets; the e1000 and virtio IRQs are wired through the PCI
interrupt line. `int 0x80` uses a DPL=3 gate and dispatches to
`syscall::dispatch` with a saved `TrapFrame`.

## Tasks and scheduling

`task.rs` owns a fixed task table. Each task has:

- a saved `TrapFrame` (ring-3 `rip/cs/rflags/rsp/ss` plus argument registers),
- an address space (`vmm::AddressSpace`),
- an fd table whose entries point into the VFS,
- a parent pid, state, exit code, cwd and brk.

`SYS_EXEC` / `SYS_EXEC_ARGV` read an ELF, create an address space, map PT_LOAD
segments, build an argv stack at the top of the user stack, and create the
task. `SYS_FORK` clones the address space contents and fd table.

The kernel main loop (`system.rs`) composes the desktop at ~60 Hz and calls
`sched::run_user_slice` between frames, which restores the next ready task's
frame and `iretq`s to ring 3. While a task runs, the 100 Hz timer IRQ
preempts it: `on_timer` round-robins to another ready task and forces a
return to the kernel every 10 ticks (`SLICE_TICKS`). A task that blocks in
`read`/`wait`/`sleep` is marked blocked and skipped. This gives user programs
real preemptive slices while keeping the GUI responsive.

## Syscalls

`syscall.rs` implements the native ABI:

- `rax` = syscall number, `rdi`, `rsi`, `rdx`, `r10` = arguments.
- User pointers are validated/copied with helpers that walk the task's page
  table (`copy_from_user`, `copy_to_user`), so a bad pointer returns `-1`
  instead of faulting the kernel.
- Standard fds 0/1/2 are created on demand as console nodes
  (`task::CONSOLE_NODE`) and writes are mirrored to COM1.

Numbers 0–41 cover process control, file I/O, directories, sockets, GUI
window buffers, theme control, polling and memory growth.

## Graphics stack

- `drivers/fb.rs` maps the Multiboot framebuffer (1024×768×32 in QEMU).
- `gfx/window.rs` owns windows: geometry, title, per-window pixel buffer,
  z-order, focus/minimise state.
- `gfx/compositor.rs` draws the wallpaper, icons, top bar, taskbar, shadows
  and window chrome into the back buffer, then blits to the framebuffer with
  a dirty flag. It also hit-tests windows/icons/taskbar/menu.
- `gfx/desktop.rs` dispatches mouse and keyboard events (press, drag,
  double-click detection, app shortcuts).
- `apps/*` implement the kernel apps on top of the window buffer API.
- Userland GUI windows use `KIND_EXTERNAL`; `SYS_GUI` lets a process create a
  window, map its framebuffer into its address space and receive mouse
  events.

## Filesystems

`fs/vfs.rs` is an arena of `Node`s. RAMFS nodes hold a `Vec<u8>`; FAT32 nodes
hold a `FatRef { first_cluster, dir_cluster, dir_index, size }` and are loaded
lazily.

`fs/ramfs.rs` creates `/bin`, `/etc`, `/home`, `/tmp`, `/disk`, `/dev`,
`/proc` and embeds the userland ELF programs and small documentation files via
`include_bytes!`.

`fs/fat32.rs` mounts the virtio-blk disk at `/disk`:

- boot-sector geometry, FAT chain walking, cluster read/write,
- 8.3 directory entries (long-name entries are skipped),
- create file/directory, unlink, truncate, append/write,
- two FAT copies updated on allocation and writes,
- case-insensitive path lookup in FAT directories.

The mount point itself becomes the FAT root node, so `/disk` lists the disk's
top-level entries rather than a synthetic child directory.

## Networking

`drivers/e1000.rs` uses MMIO + DMA rings. `net/mod.rs` implements:

- Ethernet framing, ARP cache/requests,
- IPv4 send/receive with checksums,
- ICMP echo (used by `ping`),
- UDP with DHCP client and DNS resolver,
- a minimal TCP state machine with connect/listen/accept/send/recv,
- a socket table exposed through `SYS_SOCKET`..`SYS_ACCEPT`.

`net::poll()` runs from the kernel loop to drain RX, advance timers and send
pending packets. QEMU user networking provides DHCP `10.0.2.15`, gateway
`10.0.2.2` and DNS `10.0.2.3`.

## Diagnostics

- `diag/tests.rs`: heap, PMM, VFS and FAT32 enumeration, NIC presence.
- `diag/debug_console.rs`: COM1 command line (`help ps mem fs net gfx lspci
  tasks uptime tests theme reboot halt`).
- `diag/heartbeat.rs`: watchdog fed by the main loop and drivers.
- `diag/panic.rs`: clears the screen, prints the panic location and halts.
- Terminal output is mirrored to COM1, so a headless QEMU run with
  `-serial file:serial.log` captures shell sessions for debugging.
