//! Boot-time RAMFS contents.

use crate::fs::vfs::{self, KIND_DIR, KIND_FILE};

fn write_file(dir: usize, name: &str, contents: &str) {
    if let Some(index) = vfs::create_file(dir, name) {
        vfs::write(index, 0, contents.as_bytes());
    }
}

fn write_binary(dir: usize, name: &str, contents: &[u8]) {
    if let Some(index) = vfs::create_file(dir, name) {
        vfs::write(index, 0, contents);
    }
}

/// Native ring-3 programs, compiled for the AtrodraX syscall ABI and embedded
/// into the kernel image so the system is usable without a disk.
macro_rules! user_programs {
    ($dir:expr; $($name:literal),* $(,)?) => {
        $(write_binary($dir, $name, include_bytes!(concat!("../../assets/user/", $name, ".elf")));)*
    };
}

fn mkdir(dir: usize, name: &str) -> usize {
    vfs::create_dir(dir, name).unwrap_or(dir)
}

pub fn populate() {
    let root = vfs::root();
    let bin = mkdir(root, "bin");
    let etc = mkdir(root, "etc");
    let home = mkdir(root, "home");
    let tmp = mkdir(root, "tmp");
    mkdir(root, "disk");
    let dev = mkdir(root, "dev");
    let proc = mkdir(root, "proc");
    mkdir(home, "user");

    write_file(
        root,
        "README.md",
        "# AtrodraX OS — Rust edition\n\n\
         A from-scratch x86-64 operating system written entirely in Rust.\n\n\
         Features:\n\
         - Multiboot2 boot via GRUB, long-mode entry\n\
         - GDT/IDT/TSS, PIC, PIT, PS/2 keyboard and mouse\n\
         - Bitmap physical allocator, 4-level paging, free-list heap\n\
         - Compositor desktop with themes, windows and taskbar\n\
         - VFS + RAMFS, FAT32 on virtio-blk\n\
         - Ring-3 ELF loader and preemptive round-robin scheduler\n\
         - ARP/IPv4/ICMP/UDP/TCP/DHCP/DNS networking on e1000\n\n\
         Try: help, ls, cat README.md, theme hacker, net, ping 10.0.2.2\n",
    );
    write_file(
        etc,
        "motd",
        "Welcome to AtrodraX.\nType 'help' for a list of commands.\n",
    );
    write_file(etc, "issue", "AtrodraX v0.1 (Rust)\n");
    write_file(etc, "theme", "dark\n");
    write_file(
        etc,
        "hostname",
        "atrodrax\n",
    );
    write_file(
        home,
        "todo.txt",
        "port the remaining C apps to Rust\n\
         test FAT32 writes on disk.img\n\
         run DOOM under the Linux compatibility layer\n",
    );
    write_file(dev, "null", "");

    // Linux programs that enumerate mounts (`df`, `mount`, `findmnt`) read the
    // traditional table. Everything lives in one RAMFS, so a single root entry
    // describes the whole system.
    let mounts = "ramfs / ramfs rw,relatime 0 0\nproc /proc proc rw,nosuid,nodev,noexec,relatime 0 0\n";
    write_file(proc, "mounts", mounts);
    write_file(etc, "mtab", mounts);

    user_programs!(
        bin;
        "shell",
        "cat",
        "cc",
        "counter",
        "dnsq",
        "downloader",
        "edit",
        "fetch",
        "fs_tool",
        "hello",
        "hello_linux",
        "exec_hello",
        "libc_hello",
        "busybox",
        "htmld",
        "imgview",
        "make",
        "net",
        "netmon",
        "pkg",
        "ps",
        "sleeptest",
        "spinner",
        "udpsend",
        "ui_demo",
        "browser",
    );

    let _ = (bin, tmp, dev, KIND_DIR, KIND_FILE);

    crate::kprintln!(
        "[ramfs] populated / with README, /etc, /home, /tmp, /disk, /dev, /bin ({} programs)",
        24
    );
}
