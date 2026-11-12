//! Boot-time self tests: memory, heap, VFS and disk.

use crate::fs::vfs;
use core::sync::atomic::{AtomicU32, Ordering};

static PASSED: AtomicU32 = AtomicU32::new(0);
static FAILED: AtomicU32 = AtomicU32::new(0);

fn check(name: &str, ok: bool) {
    if ok {
        PASSED.fetch_add(1, Ordering::Relaxed);
        crate::kprintln!("[test] PASS {}", name);
    } else {
        FAILED.fetch_add(1, Ordering::Relaxed);
        crate::kprintln!("[test] FAIL {}", name);
    }
}

fn heap_test() {
    let mut buffer = alloc::vec![0u8; 64 * 1024];
    for (index, byte) in buffer.iter_mut().enumerate() {
        *byte = index as u8;
    }
    let ok = buffer
        .iter()
        .enumerate()
        .all(|(index, byte)| *byte == index as u8);
    check("heap 64 KiB round trip", ok);
    drop(buffer);
}

fn pmm_test() {
    let before = crate::mem::pmm::free_pages();
    let page = crate::mem::pmm::alloc_page();
    if let Some(addr) = page {
        let virt = crate::mem::vmm::phys_to_virt(addr);
        unsafe { core::ptr::write_bytes(virt as *mut u8, 0xA5, 4096) };
        let ok = unsafe { core::ptr::read_volatile((virt + 1234) as *const u8) } == 0xA5;
        crate::mem::pmm::free_page(addr);
        let after = crate::mem::pmm::free_pages();
        check("pmm alloc/free page", ok && after == before);
    } else {
        check("pmm alloc/free page", false);
    }
}

fn vfs_test() {
    let root = vfs::root();
    let tmp = vfs::resolve("/tmp", "/").unwrap_or(root);
    let _ = vfs::unlink("/tmp/selftest.txt", "/");
    let node = match vfs::create_file(tmp, "selftest.txt") {
        Some(n) => n,
        None => {
            check("vfs create/write/read", false);
            return;
        }
    };
    vfs::write(node, 0, b"hello from the rust kernel");
    let data = vfs::read(node, 6, 4);
    let ok = data == b"from";
    let _ = vfs::unlink("/tmp/selftest.txt", "/");
    check("vfs create/write/read", ok && node < vfs::fs().nodes.len());
}

fn fat32_test() {
    if !crate::fs::fat32::mounted() {
        crate::kprintln!("[test] SKIP fat32 (no disk)");
        return;
    }
    let root = vfs::resolve("/disk", "/");
    let ok = match root {
        Some(index) => !vfs::list(index).is_empty(),
        None => false,
    };
    check("fat32 root enumerable", ok);
}

fn net_test() {
    let ok = crate::net::present();
    if ok {
        check("network link up", true);
    } else {
        crate::kprintln!("[test] SKIP network (no NIC)");
    }
}

pub fn run_all() {
    crate::kprintln!("[test] running boot self tests");
    heap_test();
    pmm_test();
    vfs_test();
    fat32_test();
    net_test();
    crate::kprintln!(
        "[test] {} passed, {} failed",
        PASSED.load(Ordering::Relaxed),
        FAILED.load(Ordering::Relaxed)
    );
}

pub fn results() -> (u32, u32) {
    (PASSED.load(Ordering::Relaxed), FAILED.load(Ordering::Relaxed))
}
