//! The default model of the `sam` provider, fetched on first use when no
//! `--sam-model` is given (feature `sam-onnx` only; nothing here runs unless
//! the provider is selected).
//!
//! Files come from the Hugging Face repository `cstr/sam2.1-hiera-tiny-ONNX`
//! into a cache directory and are compared with the SHA-256 pinned below
//! before anything uses them, so a changed file upstream is refused.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context};

use super::super::util;

pub const REPOSITORY: &str = "https://huggingface.co/cstr/sam2.1-hiera-tiny-ONNX/resolve/main";
/// Name of the model directory in the cache.
pub const NAME: &str = "sam2.1-hiera-tiny-onnx";

/// File, bytes, SHA-256: `model.json` and the two graphs it describes.
pub const FILES: [(&str, u64, &str); 3] = [
    ("model.json", 2740, "b8374809ae7f86f1e4bc8b13962f18d2e2ed45d073ce7aee6f1ac7c8857610c6"),
    ("decoder.onnx", 20_655_795, "0ae944e08e55814c19afa4d143ef07b1a3dbad2decd77c15ec3ab220b9f57247"),
    ("encoder.onnx", 109_473_919, "4fda6e68561e27808cedffa8689749a622bba5f11ba66f4ca4e3a8a9f6360db5"),
];

/// The platform's cache directory for this program: `CRISPMESH_CACHE_DIR`, else
/// `~/Library/Caches/crispmesh` (macOS), `%LOCALAPPDATA%\crispmesh` (Windows),
/// `$XDG_CACHE_HOME/crispmesh` or `~/.cache/crispmesh` (others).
pub fn cache_directory(variable: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let given = |name: &str| variable(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(directory) = given("CRISPMESH_CACHE_DIR") {
        return Some(directory);
    }
    let base = if cfg!(target_os = "macos") {
        given("HOME").map(|home| home.join("Library/Caches"))
    } else if cfg!(windows) {
        given("LOCALAPPDATA")
    } else {
        given("XDG_CACHE_HOME").or_else(|| given("HOME").map(|home| home.join(".cache")))
    };
    base.map(|base| base.join("crispmesh"))
}

/// Where the default model lives in the cache.
pub fn default_model_directory() -> Option<PathBuf> {
    cache_directory(&|name| std::env::var(name).ok()).map(|cache| cache.join("sam").join(NAME))
}

/// Whether every file is there with its pinned size (the hashes are checked before use).
pub fn present(directory: &Path) -> bool {
    FILES.iter().all(|(name, bytes, _)| std::fs::metadata(directory.join(name)).is_ok_and(|m| m.len() == *bytes))
}

/// Total bytes of the model, for progress.
pub fn total_bytes() -> u64 {
    FILES.iter().map(|f| f.1).sum()
}

/// Fetches whatever is missing or wrong into `directory`. `progress(bytes so far, total)` is called
/// while downloading and may stop it by returning an error.
pub fn ensure(directory: &Path, progress: &mut dyn FnMut(u64, u64) -> anyhow::Result<()>) -> anyhow::Result<()> {
    std::fs::create_dir_all(directory).with_context(|| directory.display().to_string())?;
    let total = total_bytes();
    let mut done = 0u64;
    for (name, bytes, sha256) in FILES {
        let target = directory.join(name);
        if std::fs::metadata(&target).is_ok_and(|m| m.len() == bytes) && util::sha256_file(&target)? == sha256 {
            done += bytes;
            progress(done, total)?;
            continue;
        }
        let partial = directory.join(format!("{name}.part"));
        let url = format!("{REPOSITORY}/{name}");
        let response = ureq::get(&url).call().map_err(|e| anyhow!("{url}: {e}"))?;
        let mut reader = response.into_body().into_reader();
        let mut file = std::fs::File::create(&partial).with_context(|| partial.display().to_string())?;
        let mut buffer = vec![0u8; 1 << 20];
        let mut written = 0u64;
        loop {
            let count = reader.read(&mut buffer).map_err(|e| anyhow!("{url}: {e}"))?;
            if count == 0 {
                break;
            }
            written += count as u64;
            if written > bytes {
                bail!("{url}: more than the expected {bytes} bytes");
            }
            file.write_all(&buffer[..count])?;
            progress(done + written, total)?;
        }
        file.sync_all()?;
        drop(file);
        let found = util::sha256_file(&partial)?;
        if written != bytes || found != sha256 {
            let _ = std::fs::remove_file(&partial);
            bail!("{url}: {written} bytes with SHA-256 {found}; expected {bytes} bytes with {sha256}");
        }
        std::fs::rename(&partial, &target)?;
        done += bytes;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_follows_the_platform_unless_told_otherwise() {
        let only = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string())
        };
        assert_eq!(cache_directory(&only(&[("CRISPMESH_CACHE_DIR", "/c"), ("HOME", "/h")])), Some(PathBuf::from("/c")));
        let home = cache_directory(&only(&[("HOME", "/h"), ("LOCALAPPDATA", "/l")]));
        let expected = if cfg!(target_os = "macos") {
            "/h/Library/Caches/crispmesh"
        } else if cfg!(windows) {
            "/l/crispmesh"
        } else {
            "/h/.cache/crispmesh"
        };
        assert_eq!(home, Some(PathBuf::from(expected)));
        assert_eq!(total_bytes(), 2740 + 20_655_795 + 109_473_919);
        // A directory with the right files of the wrong size is not taken as present.
        let folder = std::env::temp_dir().join(format!("crispmesh-sam-fetch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        for (name, _, _) in FILES {
            std::fs::write(folder.join(name), b"x").unwrap();
        }
        assert!(!present(&folder));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    /// Downloads the default model (about 130 MB): set `CRISPMESH_SAM_TEST_DOWNLOAD` to a fresh directory.
    #[test]
    fn the_default_model_downloads_and_verifies_when_asked() {
        let Some(directory) = std::env::var_os("CRISPMESH_SAM_TEST_DOWNLOAD") else {
            eprintln!("skipped: CRISPMESH_SAM_TEST_DOWNLOAD is not set");
            return;
        };
        let directory = PathBuf::from(directory);
        let mut calls = 0;
        ensure(&directory, &mut |done, total| {
            calls += 1;
            assert!(done <= total);
            Ok(())
        })
        .unwrap();
        assert!(present(&directory) && calls > 3);
        super::super::provider::verify_files(&super::super::backend::ModelInfo::read(&directory).unwrap()).unwrap();
    }
}
