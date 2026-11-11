//! Virtual filesystem: an arena of ramfs nodes plus FAT32-backed nodes.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

pub const VFS_NAME_MAX: usize = 64;
pub const KIND_FILE: u8 = 0;
pub const KIND_DIR: u8 = 1;
pub const KIND_CHAR: u8 = 2;
pub const KIND_FAT_FILE: u8 = 3;
pub const KIND_FAT_DIR: u8 = 4;

#[derive(Clone, Copy, Default)]
pub struct FatRef {
    pub first_cluster: u32,
    pub dir_cluster: u32,
    pub dir_index: usize,
    pub size: u32,
}

pub struct Node {
    pub name: String,
    pub kind: u8,
    pub data: Vec<u8>,
    pub children: Vec<usize>,
    pub parent: Option<usize>,
    pub size: usize,
    pub fat: Option<FatRef>,
    pub loaded: bool,
}

pub struct Vfs {
    pub nodes: Vec<Node>,
    pub root: usize,
}

static mut VFS: Option<Vfs> = None;

pub fn init() {
    let mut nodes = Vec::new();
    nodes.push(Node {
        name: String::from("/"),
        kind: KIND_DIR,
        data: Vec::new(),
        children: Vec::new(),
        parent: None,
        size: 0,
        fat: None,
        loaded: true,
    });
    unsafe {
        VFS = Some(Vfs { nodes, root: 0 });
    }
    crate::kprintln!("[vfs] root filesystem initialized");
}

pub fn fs() -> &'static mut Vfs {
    unsafe { VFS.as_mut().expect("vfs not initialized") }
}

pub fn root() -> usize {
    fs().root
}

pub fn node(index: usize) -> &'static mut Node {
    &mut fs().nodes[index]
}

pub fn get(index: usize) -> Option<&'static mut Node> {
    fs().nodes.get_mut(index)
}

pub fn create_node(parent: Option<usize>, name: &str, kind: u8) -> usize {
    let vfs = fs();
    let index = vfs.nodes.len();
    vfs.nodes.push(Node {
        name: name.to_string(),
        kind,
        data: Vec::new(),
        children: Vec::new(),
        parent,
        size: 0,
        fat: None,
        loaded: kind != KIND_DIR,
    });
    if let Some(parent_index) = parent {
        vfs.nodes[parent_index].children.push(index);
    }
    index
}

pub fn create_file(parent: usize, name: &str) -> Option<usize> {
    if name.is_empty() || name.len() > VFS_NAME_MAX {
        return None;
    }
    if find_child(parent, name).is_some() {
        return None;
    }
    Some(create_node(Some(parent), name, KIND_FILE))
}

pub fn create_dir(parent: usize, name: &str) -> Option<usize> {
    if name.is_empty() || name.len() > VFS_NAME_MAX {
        return None;
    }
    if find_child(parent, name).is_some() {
        return None;
    }
    Some(create_node(Some(parent), name, KIND_DIR))
}

pub fn find_child(parent: usize, name: &str) -> Option<usize> {
    let vfs = fs();
    vfs.nodes[parent]
        .children
        .iter()
        .copied()
        .find(|index| vfs.nodes[*index].name == name)
}

/// Case-insensitive lookup, used for FAT directories where the on-disk short
/// names are upper case but users usually type lower case.
pub fn find_child_ci(parent: usize, name: &str) -> Option<usize> {
    let vfs = fs();
    vfs.nodes[parent]
        .children
        .iter()
        .copied()
        .find(|index| vfs.nodes[*index].name.eq_ignore_ascii_case(name))
}

pub fn parent_of(index: usize) -> Option<usize> {
    fs().nodes[index].parent
}

pub fn name_of(index: usize) -> String {
    fs().nodes[index].name.clone()
}

pub fn path_of(index: usize) -> String {
    let vfs = fs();
    let mut parts = Vec::new();
    let mut current = Some(index);
    while let Some(i) = current {
        let node = &vfs.nodes[i];
        if node.parent.is_none() {
            break;
        }
        parts.push(node.name.clone());
        current = node.parent;
    }
    if parts.is_empty() {
        return String::from("/");
    }
    let mut result = String::new();
    for part in parts.iter().rev() {
        result.push('/');
        result.push_str(part);
    }
    result
}

#[inline]
fn is_dir(kind: u8) -> bool {
    kind == KIND_DIR || kind == KIND_FAT_DIR
}

pub fn resolve_from(start: usize, path: &str) -> Option<usize> {
    if path.is_empty() {
        return Some(start);
    }
    let mut current = if path.starts_with('/') { root() } else { start };
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            current = parent_of(current).unwrap_or(root());
            continue;
        }
        let dir_node = node(current);
        if !is_dir(dir_node.kind) {
            return None;
        }
        let is_fat_dir = dir_node.kind == KIND_FAT_DIR;
        if is_fat_dir && !dir_node.loaded {
            crate::fs::fat32::load_directory(current);
        }
        let found = if is_fat_dir {
            find_child(current, part).or_else(|| find_child_ci(current, part))
        } else {
            find_child(current, part)
        };
        match found {
            Some(child) => current = child,
            None => return None,
        }
    }
    Some(current)
}

pub fn resolve(path: &str, cwd: &str) -> Option<usize> {
    let start = if path.starts_with('/') {
        root()
    } else {
        resolve_from(root(), cwd).unwrap_or(root())
    };
    resolve_from(start, path)
}

pub fn resolve_parent(path: &str, cwd: &str) -> Option<(usize, String)> {
    let trimmed = path.trim_end_matches('/');
    let (dir_part, leaf) = match trimmed.rfind('/') {
        Some(0) => ("/", &trimmed[1..]),
        Some(pos) => (&trimmed[..pos], &trimmed[pos + 1..]),
        None => ("", trimmed),
    };
    if leaf.is_empty() {
        return None;
    }
    let dir = if dir_part.is_empty() {
        resolve_from(root(), cwd).unwrap_or(root())
    } else {
        resolve(dir_part, cwd)?
    };
    Some((dir, leaf.to_string()))
}

pub fn open(path: &str, cwd: &str) -> Option<usize> {
    resolve(path, cwd)
}

pub fn read(index: usize, offset: usize, len: usize) -> Vec<u8> {
    let node = node(index);
    match node.kind {
        KIND_FAT_FILE => crate::fs::fat32::read_file(index, offset, len),
        KIND_CHAR => Vec::new(),
        _ => {
            if offset >= node.data.len() {
                return Vec::new();
            }
            let end = (offset + len).min(node.data.len());
            node.data[offset..end].to_vec()
        }
    }
}

pub fn read_all(index: usize) -> Vec<u8> {
    let size = size_of(index);
    read(index, 0, size)
}

pub fn write(index: usize, offset: usize, data: &[u8]) -> usize {
    let node = node(index);
    match node.kind {
        KIND_FAT_FILE => crate::fs::fat32::write_file(index, offset, data),
        KIND_DIR | KIND_FAT_DIR => 0,
        _ => {
            let end = offset + data.len();
            if end > node.data.len() {
                node.data.resize(end, 0);
            }
            node.data[offset..end].copy_from_slice(data);
            node.size = node.data.len();
            data.len()
        }
    }
}

pub fn truncate(index: usize, length: usize) -> bool {
    let node = node(index);
    match node.kind {
        KIND_FAT_FILE => crate::fs::fat32::truncate_file(index, length),
        KIND_DIR | KIND_FAT_DIR => false,
        _ => {
            node.data.resize(length, 0);
            node.size = node.data.len();
            true
        }
    }
}

pub fn size_of(index: usize) -> usize {
    let node = node(index);
    if node.kind == KIND_FAT_FILE {
        node.fat.map(|f| f.size as usize).unwrap_or(0)
    } else {
        node.data.len()
    }
}

pub fn list(index: usize) -> Vec<(String, u8, usize)> {
    let node = node(index);
    if node.kind == KIND_FAT_DIR && !node.loaded {
        crate::fs::fat32::load_directory(index);
    }
    let vfs = fs();
    vfs.nodes[index]
        .children
        .iter()
        .map(|child| {
            let n = &vfs.nodes[*child];
            (n.name.clone(), n.kind, *child)
        })
        .collect()
}

pub fn unlink(path: &str, cwd: &str) -> bool {
    let Some(index) = resolve(path, cwd) else {
        return false;
    };
    if index == root() {
        return false;
    }
    let node = node(index);
    if is_dir(node.kind) {
        return false;
    }
    unsafe {
        if let Some(FatRef {
            dir_cluster,
            dir_index,
            ..
        }) = node.fat
        {
            if crate::fs::fat32::delete_entry(dir_cluster, dir_index) {
                detach(index);
                return true;
            }
            return false;
        }
    }
    detach(index);
    true
}

pub fn rmdir(path: &str, cwd: &str) -> bool {
    let Some(index) = resolve(path, cwd) else {
        return false;
    };
    if index == root() {
        return false;
    }
    let node = node(index);
    if !is_dir(node.kind) || !node.children.is_empty() {
        return false;
    }
    detach(index);
    true
}

pub fn detach(index: usize) {
    let vfs = fs();
    let parent = vfs.nodes[index].parent;
    if let Some(parent) = parent {
        vfs.nodes[parent].children.retain(|c| *c != index);
    }
}

pub fn mkdir(path: &str, cwd: &str) -> bool {
    let Some((parent, leaf)) = resolve_parent(path, cwd) else {
        return false;
    };
    if !is_dir(node(parent).kind) || find_child(parent, &leaf).is_some() {
        return false;
    }
    if node(parent).kind == KIND_FAT_DIR {
        return crate::fs::fat32::create_entry(parent, &leaf, true).is_some();
    }
    create_dir(parent, &leaf).is_some()
}

pub fn create_file_path(path: &str, cwd: &str) -> Option<usize> {
    if let Some(existing) = resolve(path, cwd) {
        return Some(existing);
    }
    let (parent, leaf) = resolve_parent(path, cwd)?;
    if !is_dir(node(parent).kind) {
        return None;
    }
    if node(parent).kind == KIND_FAT_DIR {
        return crate::fs::fat32::create_entry(parent, &leaf, false);
    }
    create_file(parent, &leaf)
}

pub fn insert_fat_node(
    parent: usize,
    name: &str,
    kind: u8,
    fat: FatRef,
    size: usize,
) -> usize {
    let index = create_node(Some(parent), name, kind);
    let node = node(index);
    node.fat = Some(fat);
    node.size = size;
    node.loaded = false;
    index
}

pub fn join_cwd(cwd: &str, path: &str) -> String {
    if path.starts_with('/') {
        return path.to_string();
    }
    if cwd == "/" {
        alloc::format!("/{}", path)
    } else {
        alloc::format!("{}/{}", cwd.trim_end_matches('/'), path)
    }
}

pub fn is_directory(index: usize) -> bool {
    is_dir(node(index).kind)
}

pub fn print_info() {
    crate::kprintln!("[vfs] {} nodes, root={}", fs().nodes.len(), root());
}
