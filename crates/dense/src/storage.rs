//! Where a run reads and writes: the file system, or a tree of files in memory.
//!
//! Every stage goes through these functions instead of `std::fs`. A path whose
//! first component is `mem:` (for example `mem:/run/events.jsonl`) names a file
//! in a process-wide in-memory tree; every other path is a file on disk. In a
//! browser (wasm32) there is no disk, so every path is in the tree. The layout
//! of a run directory and the event contract are the same in both.
//!
//! A host that has no file system puts its input files into the tree with
//! [`write`], runs with paths into the tree, and takes the outputs out with
//! [`read`] (for example when an `artifact` event names a completed file).
//! [`memory_files`] lists what a run left behind and [`remove_dir_all`] frees it.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// First path component of the in-memory tree on platforms that also have a disk.
pub const MEMORY_ROOT: &str = "mem:";

#[derive(Default)]
struct Tree {
    files: BTreeMap<String, Vec<u8>>,
    directories: BTreeSet<String>,
}

/// Looks a file up outside the tree: `(key, probe)` returns its bytes, or with
/// `probe` any `Some` when it exists.
pub type Source = Box<dyn Fn(&str, bool) -> Option<Vec<u8>> + Send + Sync>;

static SOURCE: Mutex<Option<Source>> = Mutex::new(None);

/// Registers where files that are not in the in-memory tree come from. A
/// browser host keeps large inputs (photos) in JavaScript memory and hands
/// them over one at a time, so that they do not occupy WebAssembly memory for
/// the length of a run. `None` removes the source.
pub fn set_memory_source(source: Option<Source>) {
    *SOURCE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = source;
}

fn from_source(key: &str, probe: bool) -> Option<Vec<u8>> {
    SOURCE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref().and_then(|source| source(key, probe))
}

/// Lists a directory outside the tree: `key` returns the names of the entries
/// directly inside it, or `None` when the source has no such directory.
pub type Lister = Box<dyn Fn(&str) -> Option<Vec<String>> + Send + Sync>;

static LISTER: Mutex<Option<Lister>> = Mutex::new(None);

/// Registers how directories of the memory source are listed (a folder of
/// photos handed over lazily is read by name, so it has to be listable).
/// `None` removes it.
pub fn set_memory_lister(lister: Option<Lister>) {
    *LISTER.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = lister;
}

fn from_lister(key: &str) -> Option<Vec<String>> {
    LISTER.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref().and_then(|lister| lister(key))
}

static TREE: Mutex<Tree> = Mutex::new(Tree { files: BTreeMap::new(), directories: BTreeSet::new() });

fn tree() -> MutexGuard<'static, Tree> {
    TREE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// True when `path` names a file of the in-memory tree.
pub fn is_memory(path: &Path) -> bool {
    cfg!(target_arch = "wasm32") || path.components().next().is_some_and(|c| c.as_os_str() == MEMORY_ROOT)
}

/// Key of a path in the tree: forward slashes, no `.` or empty components, no trailing slash.
fn key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let parts: Vec<&str> = text.split('/').filter(|part| !part.is_empty() && *part != ".").collect();
    parts.join("/")
}

fn missing(path: &Path) -> Error {
    Error::new(ErrorKind::NotFound, format!("no such file in memory: {}", path.display()))
}

impl Tree {
    fn is_dir(&self, key: &str) -> bool {
        let prefix = format!("{key}/");
        self.directories.contains(key)
            || self.files.range(prefix.clone()..).next().is_some_and(|(k, _)| k.starts_with(&prefix))
            || self.directories.range(prefix.clone()..).next().is_some_and(|k| k.starts_with(&prefix))
    }
}

pub fn read(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    if !is_memory(path) {
        return std::fs::read(path);
    }
    let key = key(path);
    let stored = tree().files.get(&key).cloned();
    stored.or_else(|| from_source(&key, false)).ok_or_else(|| missing(path))
}

pub fn read_to_string(path: impl AsRef<Path>) -> Result<String> {
    String::from_utf8(read(path)?).map_err(|e| Error::new(ErrorKind::InvalidData, e))
}

pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<()> {
    let path = path.as_ref();
    if !is_memory(path) {
        return std::fs::write(path, contents);
    }
    tree().files.insert(key(path), contents.as_ref().to_vec());
    Ok(())
}

/// Stores a buffer without copying it when the target is in memory.
pub fn write_owned(path: impl AsRef<Path>, contents: Vec<u8>) -> Result<()> {
    let path = path.as_ref();
    if !is_memory(path) {
        return std::fs::write(path, contents);
    }
    tree().files.insert(key(path), contents);
    Ok(())
}

/// Like `OpenOptions::new().write(true).create_new(true)`: fails when the file exists.
pub fn write_new(path: impl AsRef<Path>, contents: Vec<u8>) -> Result<()> {
    let path = path.as_ref();
    if !is_memory(path) {
        use std::io::Write;
        return std::fs::OpenOptions::new().write(true).create_new(true).open(path)?.write_all(&contents);
    }
    let mut tree = tree();
    if tree.files.contains_key(&key(path)) {
        return Err(Error::new(ErrorKind::AlreadyExists, format!("{} exists", path.display())));
    }
    tree.files.insert(key(path), contents);
    Ok(())
}

/// Appends to a file, creating it if needed. On disk the bytes are written with one call and synced.
pub fn append(path: impl AsRef<Path>, contents: &[u8]) -> Result<()> {
    let path = path.as_ref();
    if !is_memory(path) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(contents)?;
        return file.sync_all();
    }
    tree().files.entry(key(path)).or_default().extend_from_slice(contents);
    Ok(())
}

pub fn exists(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    if !is_memory(path) {
        return path.exists();
    }
    let key = key(path);
    let stored = {
        let tree = tree();
        tree.files.contains_key(&key) || tree.is_dir(&key)
    };
    stored || from_source(&key, true).is_some() || from_lister(&key).is_some()
}

pub fn is_file(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    if !is_memory(path) {
        return path.is_file();
    }
    let key = key(path);
    let stored = tree().files.contains_key(&key);
    stored || from_source(&key, true).is_some()
}

pub fn is_dir(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    if !is_memory(path) {
        return path.is_dir();
    }
    let key = key(path);
    let stored = tree().is_dir(&key);
    stored || from_lister(&key).is_some()
}

/// Fails when the directory exists, like `std::fs::create_dir`.
pub fn create_dir(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    if !is_memory(path) {
        return std::fs::create_dir(path);
    }
    if exists(path) {
        return Err(Error::new(ErrorKind::AlreadyExists, format!("{} exists", path.display())));
    }
    tree().directories.insert(key(path));
    Ok(())
}

pub fn create_dir_all(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    if !is_memory(path) {
        return std::fs::create_dir_all(path);
    }
    tree().directories.insert(key(path));
    Ok(())
}

pub fn remove_file(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    if !is_memory(path) {
        return std::fs::remove_file(path);
    }
    tree().files.remove(&key(path)).map(|_| ()).ok_or_else(|| missing(path))
}

pub fn remove_dir_all(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    if !is_memory(path) {
        return std::fs::remove_dir_all(path);
    }
    let (mut tree, key) = (tree(), key(path));
    let prefix = format!("{key}/");
    tree.files.retain(|k, _| !k.starts_with(&prefix));
    tree.directories.retain(|k| k != &key && !k.starts_with(&prefix));
    Ok(())
}

pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<()> {
    let (from, to) = (from.as_ref(), to.as_ref());
    if !is_memory(from) {
        return std::fs::rename(from, to);
    }
    let mut tree = tree();
    let contents = tree.files.remove(&key(from)).ok_or_else(|| missing(from))?;
    tree.files.insert(key(to), contents);
    Ok(())
}

/// Paths of the entries directly inside a directory, sorted.
pub fn list(directory: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let directory = directory.as_ref();
    if !is_memory(directory) {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(directory)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
        entries.sort();
        return Ok(entries);
    }
    let key = key(directory);
    let mut names: BTreeSet<String> = from_lister(&key).unwrap_or_default().into_iter().collect();
    let tree = tree();
    let prefix = format!("{key}/");
    for path in tree.files.keys().chain(tree.directories.iter()).filter(|k| k.starts_with(&prefix)) {
        names.insert(path[prefix.len()..].split('/').next().unwrap_or_default().to_string());
    }
    Ok(names.into_iter().filter(|n| !n.is_empty()).map(|n| directory.join(n)).collect())
}

/// `std::path::absolute` on disk; paths of the in-memory tree are returned as they are.
pub fn absolute(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    if !is_memory(path) {
        return std::path::absolute(path);
    }
    Ok(path.to_path_buf())
}

/// `std::fs::canonicalize` on disk; paths of the in-memory tree are returned as they are.
pub fn canonicalize(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    if !is_memory(path) {
        return std::fs::canonicalize(path);
    }
    if exists(path) {
        Ok(path.to_path_buf())
    } else {
        Err(missing(path))
    }
}

/// Every file of the in-memory tree under `directory` (recursively) with its size, sorted by path.
pub fn memory_files(directory: impl AsRef<Path>) -> Vec<(String, usize)> {
    let key = key(directory.as_ref());
    let prefix = if key.is_empty() { String::new() } else { format!("{key}/") };
    tree().files.iter().filter(|(k, _)| k.starts_with(&prefix)).map(|(k, v)| (k.clone(), v.len())).collect()
}

/// Bytes held by the in-memory tree.
pub fn memory_bytes() -> usize {
    tree().files.values().map(Vec::len).sum()
}

/// Decodes an image file (PNG or JPEG).
pub fn open_image(path: impl AsRef<Path>) -> anyhow::Result<image::DynamicImage> {
    let path = path.as_ref();
    let bytes = read(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    image::load_from_memory(&bytes).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

/// Writes an 8-bit image with 1 (grey) or 3 (RGB) channels as PNG.
pub fn save_png(path: impl AsRef<Path>, width: usize, height: usize, channels: usize, data: &[u8]) -> anyhow::Result<()> {
    let bytes = png_bytes(width, height, channels, data)?;
    write_owned(path.as_ref(), bytes).map_err(|e| anyhow::anyhow!("{}: {e}", path.as_ref().display()))
}

/// Encodes an 8-bit grey or RGB image without a temporary file.
pub fn png_bytes(width: usize, height: usize, channels: usize, data: &[u8]) -> anyhow::Result<Vec<u8>> {
    use image::ImageEncoder;
    let colour = match channels {
        1 => image::ExtendedColorType::L8,
        3 => image::ExtendedColorType::Rgb8,
        other => anyhow::bail!("cannot write an image with {other} channels"),
    };
    anyhow::ensure!(data.len() == width * height * channels, "image data does not match its size");
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes).write_image(data, width as u32, height as u32, colour)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_memory_tree_behaves_like_a_directory() {
        let root = PathBuf::from(format!("mem:/storage-test-{}", std::process::id()));
        assert!(is_memory(&root) && !is_memory(Path::new("/tmp/x")) && !is_memory(Path::new("memory/x")));
        assert!(!exists(&root));
        create_dir_all(root.join("run/stereo")).unwrap();
        assert!(is_dir(&root) && is_dir(root.join("run")) && exists(root.join("run/stereo")) && !is_file(root.join("run")));
        assert!(create_dir(root.join("run")).is_err());
        write(root.join("run/config.json"), b"{}").unwrap();
        append(root.join("run/events.jsonl"), b"a\n").unwrap();
        append(root.join("run/events.jsonl"), b"b\n").unwrap();
        assert_eq!(read_to_string(root.join("run/events.jsonl")).unwrap(), "a\nb\n");
        assert!(write_new(root.join("run/config.json"), vec![1]).is_err());
        write_new(root.join("run/mesh/mesh.stl"), vec![1, 2, 3]).unwrap();
        assert!(is_dir(root.join("run/mesh")), "a file implies its directories");
        rename(root.join("run/mesh/mesh.stl"), root.join("run/mesh/final.stl")).unwrap();
        assert!(read(root.join("run/mesh/mesh.stl")).is_err());
        let names: Vec<String> =
            list(root.join("run")).unwrap().iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["config.json", "events.jsonl", "mesh", "stereo"]);
        assert_eq!(memory_files(&root).len(), 3);
        assert_eq!(canonicalize(root.join("run")).unwrap(), root.join("run"));
        assert_eq!(absolute(root.join("x")).unwrap(), root.join("x"));
        remove_file(root.join("run/config.json")).unwrap();
        assert!(remove_file(root.join("run/config.json")).is_err());
        remove_dir_all(&root).unwrap();
        assert!(!exists(&root) && memory_files(&root).is_empty());
    }

    #[test]
    fn a_source_supplies_files_that_are_not_stored() {
        // One process-wide source: this is the only test that sets it.
        set_memory_source(Some(Box::new(|key, probe| {
            (key == "mem:/source-test/photo.png").then(|| if probe { Vec::new() } else { vec![7, 8, 9] })
        })));
        let path = Path::new("mem:/source-test/photo.png");
        assert!(exists(path) && is_file(path));
        assert_eq!(read(path).unwrap(), vec![7, 8, 9]);
        assert!(!exists("mem:/source-test/other.png") && read("mem:/source-test/other.png").is_err());
        write(path, [1]).unwrap();
        assert_eq!(read(path).unwrap(), vec![1], "a stored file wins");
        remove_file(path).unwrap();
        // A lister makes the source's folder a directory with its entries, next to stored ones.
        set_memory_lister(Some(Box::new(|key| (key == "mem:/source-test").then(|| vec!["photo.png".to_string()]))));
        write("mem:/source-test/stored.png", [2]).unwrap();
        assert!(is_dir("mem:/source-test") && exists("mem:/source-test"));
        let names: Vec<String> =
            list("mem:/source-test").unwrap().iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["photo.png", "stored.png"]);
        remove_file("mem:/source-test/stored.png").unwrap();
        set_memory_lister(None);
        set_memory_source(None);
        assert!(!exists(path) && !is_dir("mem:/source-test"));
    }

    #[test]
    fn images_round_trip_in_memory_and_on_disk() {
        let pixels: Vec<u8> = (0..4 * 3 * 3).map(|n| (n * 7) as u8).collect();
        let disk = std::env::temp_dir().join(format!("crispmesh-storage-{}.png", std::process::id()));
        for path in [PathBuf::from(format!("mem:/storage-image-{}.png", std::process::id())), disk] {
            save_png(&path, 4, 3, 3, &pixels).unwrap();
            assert_eq!(open_image(&path).unwrap().to_rgb8().into_raw(), pixels);
            save_png(&path, 4, 3, 1, &pixels[..12]).unwrap();
            assert_eq!(open_image(&path).unwrap().to_luma8().into_raw(), &pixels[..12]);
            remove_file(&path).unwrap();
        }
        assert!(save_png("mem:/x.png", 2, 2, 3, &[0; 5]).is_err());
    }
}
