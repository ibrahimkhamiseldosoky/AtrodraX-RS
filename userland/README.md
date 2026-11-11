# Userland

The ring-3 programs shipped with AtrodraX-RS are the prebuilt AtrodraX ELF
binaries. They are embedded into the kernel RAMFS from
`kernel/assets/user/*.elf` and installed under `/bin` at boot:

```
shell cat cc counter dnsq downloader edit fetch fs_tool hello htmld
imgview make net netmon pkg ps sleeptest spinner udpsend ui_demo browser
```

Each binary talks to the kernel through the native `int 0x80` syscall ABI
(`rax` = number, `rdi/rsi/rdx/r10` = arguments), which is implemented in
`kernel/src/syscall.rs`. Sources for these programs live in the upstream
AtrodraX repository (`userspace/`); this directory is reserved for a future
Rust userland toolchain and is intentionally empty for now.

To rebuild the kernel with different binaries, replace the ELFs in
`kernel/assets/user/` and rebuild the ISO. The RAMFS macro in
`kernel/src/fs/ramfs.rs` lists every embedded program explicitly.
