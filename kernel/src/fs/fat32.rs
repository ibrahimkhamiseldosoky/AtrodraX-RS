//! FAT32 driver backed by the virtio-blk disk.
//!
//! The mount is exposed as `/disk`. Directory loading is lazy: a FAT
//! directory node is populated the first time it is listed or traversed.

use crate::drivers::virtio_blk;
use crate::fs::vfs::{self, FatRef, KIND_FAT_DIR, KIND_FAT_FILE};
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Default)]
pub struct Fat32 {
    pub mounted: bool,
    pub bytes_per_sector: u32,
    pub sectors_per_cluster: u32,
    pub reserved_sectors: u32,
    pub num_fats: u32,
    pub fat_size: u32,
    pub root_cluster: u32,
    pub total_sectors: u32,
    pub fat_start: u32,
    pub data_start: u32,
    pub cluster_count: u32,
}

static mut FS: Fat32 = Fat32 {
    mounted: false,
    bytes_per_sector: 0,
    sectors_per_cluster: 0,
    reserved_sectors: 0,
    num_fats: 0,
    fat_size: 0,
    root_cluster: 0,
    total_sectors: 0,
    fat_start: 0,
    data_start: 0,
    cluster_count: 0,
};

pub fn info() -> Fat32 {
    unsafe { FS }
}

pub fn mounted() -> bool {
    unsafe { FS.mounted }
}

fn read_sector(lba: u32, buf: &mut [u8]) -> bool {
    virtio_blk::read_sector(lba as u64, buf)
}

fn write_sector(lba: u32, buf: &[u8]) -> bool {
    virtio_blk::write_sector(lba as u64, buf)
}

pub fn mount() {
    if !virtio_blk::present() {
        crate::kprintln!("[fat32] no virtio-blk disk; skipping mount");
        return;
    }
    let mut sector = [0u8; 512];
    if !read_sector(0, &mut sector) {
        crate::kprintln!("[fat32] boot sector read failed");
        return;
    }
    if &sector[82..87] != b"FAT32" {
        crate::kprintln!("[fat32] no FAT32 signature on disk");
        return;
    }
    let bytes_per_sector = u16::from_le_bytes([sector[11], sector[12]]) as u32;
    let sectors_per_cluster = sector[13] as u32;
    let reserved = u16::from_le_bytes([sector[14], sector[15]]) as u32;
    let num_fats = sector[16] as u32;
    let total16 = u16::from_le_bytes([sector[19], sector[20]]) as u32;
    let total32 = u32::from_le_bytes([sector[32], sector[33], sector[34], sector[35]]);
    let total = if total16 != 0 { total16 } else { total32 };
    let fat_size = u32::from_le_bytes([sector[36], sector[37], sector[38], sector[39]]);
    let root_cluster = u32::from_le_bytes([sector[44], sector[45], sector[46], sector[47]]);
    let data_start = reserved + num_fats * fat_size;
    let cluster_count = if sectors_per_cluster != 0 {
        (total.saturating_sub(data_start)) / sectors_per_cluster
    } else {
        0
    };

    if bytes_per_sector != 512 || sectors_per_cluster == 0 || fat_size == 0 {
        crate::kprintln!("[fat32] unsupported geometry");
        return;
    }

    unsafe {
        FS = Fat32 {
            mounted: true,
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors: reserved,
            num_fats,
            fat_size,
            root_cluster,
            total_sectors: total,
            fat_start: reserved,
            data_start,
            cluster_count,
        };
    }

    // Turn the existing `/disk` mount point into the FAT32 root itself.  The
    // directory is kept so that paths such as `/disk/README.TXT` resolve as
    // expected, but it is no longer a RAMFS directory containing a nested `/`.
    let disk_root = vfs::resolve("/disk", "/")
        .or_else(|| vfs::create_dir(vfs::root(), "disk"))
        .unwrap();
    {
        let node = vfs::get(disk_root).unwrap();
        node.kind = KIND_FAT_DIR;
        node.children.clear();
        node.data.clear();
        node.size = 0;
        node.fat = Some(FatRef {
            first_cluster: root_cluster,
            dir_cluster: root_cluster,
            dir_index: 0,
            size: 0,
        });
        node.loaded = false;
    }

    crate::kprintln!(
        "[fat32] mounted {} MiB, {}/cluster, root cluster {}",
        (total as u64 * 512) / (1024 * 1024),
        sectors_per_cluster,
        root_cluster
    );
}

fn cluster_to_lba(cluster: u32) -> u32 {
    let fs = info();
    fs.data_start + (cluster - 2) * fs.sectors_per_cluster
}

fn fat_next(cluster: u32) -> Option<u32> {
    let fs = info();
    let offset = cluster * 4;
    let sector = fs.fat_start + offset / fs.bytes_per_sector;
    let index = (offset % fs.bytes_per_sector) as usize;
    let mut buf = [0u8; 512];
    if !read_sector(sector, &mut buf) {
        return None;
    }
    let next = u32::from_le_bytes([buf[index], buf[index + 1], buf[index + 2], buf[index + 3]]) & 0x0FFF_FFFF;
    if next >= 0x0FFF_FFF8 {
        None
    } else {
        Some(next)
    }
}

fn fat_set(cluster: u32, value: u32) -> bool {
    let fs = info();
    let offset = cluster * 4;
    let sector = fs.fat_start + offset / fs.bytes_per_sector;
    let index = (offset % fs.bytes_per_sector) as usize;
    let mut buf = [0u8; 512];
    if !read_sector(sector, &mut buf) {
        return false;
    }
    let old = u32::from_le_bytes([buf[index], buf[index + 1], buf[index + 2], buf[index + 3]]);
    let new = (old & 0xF000_0000) | (value & 0x0FFF_FFFF);
    buf[index..index + 4].copy_from_slice(&new.to_le_bytes());
    for fat in 0..fs.num_fats {
        let lba = fs.fat_start + fat * fs.fat_size + offset / fs.bytes_per_sector;
        if !write_sector(lba, &buf) {
            return false;
        }
    }
    true
}

fn read_cluster(cluster: u32, buf: &mut [u8]) -> bool {
    let fs = info();
    let lba = cluster_to_lba(cluster);
    for s in 0..fs.sectors_per_cluster {
        let start = (s * 512) as usize;
        if start + 512 > buf.len() {
            break;
        }
        if !read_sector(lba + s, &mut buf[start..start + 512]) {
            return false;
        }
    }
    true
}

fn write_cluster(cluster: u32, buf: &[u8]) -> bool {
    let fs = info();
    let lba = cluster_to_lba(cluster);
    for s in 0..fs.sectors_per_cluster {
        let start = (s * 512) as usize;
        let end = start + 512;
        if start >= buf.len() {
            break;
        }
        let mut sector = [0u8; 512];
        let copy = (end.min(buf.len())) - start;
        sector[..copy].copy_from_slice(&buf[start..start + copy]);
        if !write_sector(lba + s, &sector) {
            return false;
        }
    }
    true
}

fn alloc_cluster() -> Option<u32> {
    let fs = info();
    for cluster in 2..fs.cluster_count + 2 {
        let offset = cluster * 4;
        let sector = fs.fat_start + offset / fs.bytes_per_sector;
        let index = (offset % fs.bytes_per_sector) as usize;
        let mut buf = [0u8; 512];
        if !read_sector(sector, &mut buf) {
            return None;
        }
        let value = u32::from_le_bytes([buf[index], buf[index + 1], buf[index + 2], buf[index + 3]]);
        if value & 0x0FFF_FFFF == 0 {
            fat_set(cluster, 0x0FFF_FFFF);
            return Some(cluster);
        }
    }
    None
}

fn short_name(name: &str) -> [u8; 11] {
    let mut result = [b' '; 11];
    let upper = name.to_ascii_uppercase();
    let (base, ext) = match upper.rfind('.') {
        Some(pos) => (&upper[..pos], &upper[pos + 1..]),
        None => (upper.as_str(), ""),
    };
    for (i, b) in base.bytes().take(8).enumerate() {
        result[i] = b;
    }
    for (i, b) in ext.bytes().take(3).enumerate() {
        result[8 + i] = b;
    }
    result
}

fn entry_name(entry: &[u8]) -> Option<String> {
    if entry[0] == 0x00 || entry[0] == 0xE5 {
        return None;
    }
    let attr = entry[11];
    if attr & 0x08 != 0 {
        return None;
    }
    // Long-file-name entries are skipped; short names are good enough.
    let mut name = String::new();
    for i in 0..8 {
        let b = entry[i];
        if b == b' ' {
            break;
        }
        name.push(b as char);
    }
    let mut ext = String::new();
    for i in 8..11 {
        let b = entry[i];
        if b == b' ' {
            break;
        }
        ext.push(b as char);
    }
    if !ext.is_empty() {
        name.push('.');
        name.push_str(&ext);
    }
    Some(name)
}

pub fn load_directory(vfs_index: usize) {
    // Copy the FAT reference instead of holding a `&mut Node` across
    // `insert_fat_node`: inserting can reallocate the VFS node vector, which
    // would invalidate the reference.
    let fat = match vfs::get(vfs_index) {
        Some(n) => match n.fat {
            Some(f) => f,
            None => return,
        },
        None => return,
    };
    let mut cluster = fat.first_cluster;
    let mut guard = 0;
    let mut buf = alloc::vec![0u8; 4096];
    loop {
        if guard > 4096 {
            break;
        }
        guard += 1;
        if !read_cluster(cluster, &mut buf) {
            break;
        }
        let entries = (info().sectors_per_cluster as usize * 512) / 32;
        for index in 0..entries {
            let offset = index * 32;
            let entry = &buf[offset..offset + 32];
            if entry[0] == 0x00 {
                if let Some(node) = vfs::get(vfs_index) {
                    node.loaded = true;
                }
                return;
            }
            if let Some(name) = entry_name(entry) {
                if name == "." || name == ".." {
                    continue;
                }
                let attr = entry[11];
                let first = ((entry[20] as u32) << 16)
                    | ((entry[21] as u32) << 24)
                    | entry[26] as u32
                    | ((entry[27] as u32) << 8);
                let size = u32::from_le_bytes([entry[28], entry[29], entry[30], entry[31]]);
                let kind = if attr & 0x10 != 0 {
                    KIND_FAT_DIR
                } else {
                    KIND_FAT_FILE
                };
                if vfs::find_child(vfs_index, &name).is_none() {
                    vfs::insert_fat_node(
                        vfs_index,
                        &name,
                        kind,
                        FatRef {
                            first_cluster: first,
                            dir_cluster: cluster,
                            dir_index: index,
                            size,
                        },
                        size as usize,
                    );
                }
            }
        }
        match fat_next(cluster) {
            Some(next) => cluster = next,
            None => break,
        }
    }
    if let Some(node) = vfs::get(vfs_index) {
        node.loaded = true;
    }
}

pub fn read_file(vfs_index: usize, offset: usize, len: usize) -> Vec<u8> {
    let node = match vfs::get(vfs_index) {
        Some(n) => n,
        None => return Vec::new(),
    };
    let fat = match node.fat {
        Some(f) => f,
        None => return Vec::new(),
    };
    let size = fat.size as usize;
    if offset >= size {
        return Vec::new();
    }
    let end = (offset + len).min(size);
    let mut result = Vec::with_capacity(end - offset);
    let mut cluster = fat.first_cluster;
    let cluster_size = (info().sectors_per_cluster * 512) as usize;
    let mut cluster_start = 0usize;
    let mut buf = alloc::vec![0u8; cluster_size.max(512)];
    while cluster_start < end {
        let cluster_end = cluster_start + cluster_size;
        if cluster_end > offset {
            if !read_cluster(cluster, &mut buf) {
                break;
            }
            let from = offset.max(cluster_start) - cluster_start;
            let to = end.min(cluster_end) - cluster_start;
            result.extend_from_slice(&buf[from..to]);
        }
        cluster_start = cluster_end;
        match fat_next(cluster) {
            Some(next) => cluster = next,
            None => break,
        }
    }
    result
}

/// Write back a directory entry's first-cluster and size fields.
///
/// `create_entry` writes a zero size, and FAT has no separate inode, so every
/// size-changing operation must update the owning directory entry or the file
/// comes back empty after the next mount.
fn update_dirent(fat: &FatRef, size: u32) {
    if fat.dir_cluster < 2 {
        return;
    }
    let cluster_size = (info().sectors_per_cluster * 512) as usize;
    let mut buf = alloc::vec![0u8; cluster_size];
    if !read_cluster(fat.dir_cluster, &mut buf) {
        return;
    }
    let offset = fat.dir_index * 32;
    if offset + 32 > buf.len() {
        return;
    }
    let first = fat.first_cluster;
    buf[offset + 20] = (first >> 16) as u8;
    buf[offset + 21] = (first >> 24) as u8;
    buf[offset + 26] = first as u8;
    buf[offset + 27] = (first >> 8) as u8;
    buf[offset + 28..offset + 32].copy_from_slice(&size.to_le_bytes());
    write_cluster(fat.dir_cluster, &buf);
}

pub fn write_file(vfs_index: usize, offset: usize, data: &[u8]) -> usize {
    let node = match vfs::get(vfs_index) {
        Some(n) => n,
        None => return 0,
    };
    let mut fat = match node.fat {
        Some(f) => f,
        None => return 0,
    };
    let cluster_size = (info().sectors_per_cluster * 512) as usize;
    let needed_clusters = (offset + data.len() + cluster_size - 1) / cluster_size;

    let mut chain = Vec::new();
    let mut cluster = fat.first_cluster;
    while cluster >= 2 && cluster < 0x0FFF_FFF8 {
        chain.push(cluster);
        match fat_next(cluster) {
            Some(next) => cluster = next,
            None => break,
        }
    }
    while chain.len() < needed_clusters {
        let new_cluster = match alloc_cluster() {
            Some(c) => c,
            None => return 0,
        };
        if let Some(last) = chain.last() {
            fat_set(*last, new_cluster);
        }
        chain.push(new_cluster);
    }
    if fat.first_cluster < 2 && !chain.is_empty() {
        fat.first_cluster = chain[0];
    }

    let mut written = 0usize;
    let mut pos = offset;
    while written < data.len() {
        let cluster_index = pos / cluster_size;
        let in_cluster = pos % cluster_size;
        if cluster_index >= chain.len() {
            break;
        }
        let target = chain[cluster_index];
        let mut buf = alloc::vec![0u8; cluster_size];
        read_cluster(target, &mut buf);
        let chunk = (cluster_size - in_cluster).min(data.len() - written);
        buf[in_cluster..in_cluster + chunk].copy_from_slice(&data[written..written + chunk]);
        if !write_cluster(target, &buf) {
            break;
        }
        written += chunk;
        pos += chunk;
    }

    fat.size = ((offset + written) as u32).max(fat.size);
    node.fat = Some(fat);
    node.size = fat.size as usize;
    update_dirent(&fat, fat.size);
    written
}

pub fn truncate_file(vfs_index: usize, length: usize) -> bool {
    // Simplification: truncate to zero frees the chain; otherwise the size
    // field is updated without reclaiming trailing clusters.
    let node = match vfs::get(vfs_index) {
        Some(n) => n,
        None => return false,
    };
    let mut fat = match node.fat {
        Some(f) => f,
        None => return false,
    };
    if length == 0 {
        let mut cluster = fat.first_cluster;
        let mut guard = 0;
        while cluster >= 2 && cluster < 0x0FFF_FFF8 && guard < 65536 {
            guard += 1;
            let next = fat_next(cluster);
            fat_set(cluster, 0);
            match next {
                Some(n) => cluster = n,
                None => break,
            }
        }
        fat.first_cluster = 0;
        fat.size = 0;
    } else {
        fat.size = length as u32;
    }
    node.fat = Some(fat);
    node.size = fat.size as usize;
    update_dirent(&fat, fat.size);
    true
}

pub fn delete_entry(dir_cluster: u32, index: usize) -> bool {
    let lba = cluster_to_lba(dir_cluster);
    let entries_per_sector = 512 / 32;
    let sector = lba + (index / entries_per_sector) as u32;
    let offset = (index % entries_per_sector) * 32;
    let mut buf = [0u8; 512];
    if !read_sector(sector, &mut buf) {
        return false;
    }
    buf[offset] = 0xE5;
    write_sector(sector, &buf)
}

pub fn create_entry(parent: usize, name: &str, directory: bool) -> Option<usize> {
    let node = vfs::get(parent)?;
    let fat = node.fat?;
    let short = short_name(name);

    let mut cluster = fat.first_cluster;
    let mut guard = 0;
    let mut buf = alloc::vec![0u8; 4096];
    loop {
        guard += 1;
        if guard > 4096 {
            return None;
        }
        if !read_cluster(cluster, &mut buf) {
            return None;
        }
        let entries = (info().sectors_per_cluster as usize * 512) / 32;
        for index in 0..entries {
            let offset = index * 32;
            let entry = &mut buf[offset..offset + 32];
            if entry[0] == 0x00 || entry[0] == 0xE5 {
                let first = if directory { alloc_cluster()? } else { 0 };
                for b in entry.iter_mut() {
                    *b = 0;
                }
                entry[..11].copy_from_slice(&short);
                entry[11] = if directory { 0x10 } else { 0x20 };
                entry[20] = (first >> 16) as u8;
                entry[21] = (first >> 24) as u8;
                entry[26] = first as u8;
                entry[27] = (first >> 8) as u8;
                if !write_cluster(cluster, &buf) {
                    return None;
                }
                if directory {
                    // Write "." and ".." entries in the new cluster.
                    let mut dir_buf = alloc::vec![0u8; (info().sectors_per_cluster * 512) as usize];
                    for b in dir_buf.iter_mut() {
                        *b = 0;
                    }
                    dir_buf[..11].copy_from_slice(b".          ");
                    dir_buf[11] = 0x10;
                    dir_buf[20] = (first >> 16) as u8;
                    dir_buf[21] = (first >> 24) as u8;
                    dir_buf[26] = first as u8;
                    dir_buf[27] = (first >> 8) as u8;
                    dir_buf[32..43].copy_from_slice(b"..         ");
                    dir_buf[43] = 0x10;
                    dir_buf[52] = (fat.first_cluster >> 16) as u8;
                    dir_buf[53] = (fat.first_cluster >> 24) as u8;
                    dir_buf[58] = fat.first_cluster as u8;
                    dir_buf[59] = (fat.first_cluster >> 8) as u8;
                    write_cluster(first, &dir_buf);
                }
                let kind = if directory { KIND_FAT_DIR } else { KIND_FAT_FILE };
                return Some(vfs::insert_fat_node(
                    parent,
                    name,
                    kind,
                    FatRef {
                        first_cluster: first,
                        dir_cluster: cluster,
                        dir_index: index,
                        size: 0,
                    },
                    0,
                ));
            }
        }
        match fat_next(cluster) {
            Some(next) => cluster = next,
            None => return None,
        }
    }
}
