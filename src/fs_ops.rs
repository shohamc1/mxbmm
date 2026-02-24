use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use notify::{RecursiveMode, Watcher};
use walkdir::WalkDir;
use zip::ZipArchive;

use crate::model::{FsWatcherState, InstallTarget, ModEntry};

pub fn default_mods_root() -> PathBuf {
    if let Ok(path) = std::env::var("MXBMM_MODS_ROOT") {
        return PathBuf::from(path);
    }

    if let Some(documents) = dirs::document_dir() {
        return documents.join("PiBoSo").join("MX Bikes").join("mods");
    }

    PathBuf::from(".").join("mods")
}

pub fn read_mod_entries(dir: &Path, excluded_dir_names: &[&str]) -> Vec<ModEntry> {
    let mut entries = Vec::new();
    let read_dir = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return entries,
    };

    for item in read_dir.flatten() {
        let name = item.file_name().to_string_lossy().to_string();
        let path = item.path();
        if path.is_dir()
            && excluded_dir_names
                .iter()
                .any(|excluded| excluded.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        if !path.is_dir() && !is_pkz_file(&path) && !is_pnt_file(&path) {
            continue;
        }
        entries.push(ModEntry { name, path });
    }

    entries.sort_by_key(|e| e.name.to_lowercase());
    entries
}

pub fn is_supported_archive(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("zip"))
        .unwrap_or(false)
}

pub fn is_pkz_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("pkz"))
        .unwrap_or(false)
}

pub fn is_pnt_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("pnt"))
        .unwrap_or(false)
}

pub fn with_extension_if_missing(name: &str, extension: &str) -> String {
    if name.to_lowercase().ends_with(extension) {
        name.to_string()
    } else {
        format!("{name}{extension}")
    }
}

pub fn extract_zip_archive(archive_path: &Path, destination: &Path) -> io::Result<()> {
    let file = File::open(archive_path)?;
    let mut archive = ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        let Some(enclosed_name) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };

        let outpath = destination.join(enclosed_name);
        if entry.name().ends_with('/') {
            fs::create_dir_all(&outpath)?;
            continue;
        }

        if let Some(parent) = outpath.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(&outpath)?;
        io::copy(&mut entry, &mut output)?;
    }

    Ok(())
}

pub fn guess_mod_name(extract_dir: &Path, archive_path: &Path) -> String {
    let mut entries = match fs::read_dir(extract_dir) {
        Ok(read_dir) => read_dir.flatten().collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };

    if entries.len() == 1 {
        let first = entries.remove(0);
        if first.path().is_dir() {
            return first.file_name().to_string_lossy().to_string();
        }
    }

    archive_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mod")
        .to_string()
}

pub fn pick_source_root(extract_dir: &Path) -> PathBuf {
    let mut entries = match fs::read_dir(extract_dir) {
        Ok(read_dir) => read_dir.flatten().collect::<Vec<_>>(),
        Err(_) => return extract_dir.to_path_buf(),
    };

    if entries.len() == 1 {
        let entry = entries.remove(0);
        let path = entry.path();
        if path.is_dir() {
            return path;
        }
    }

    extract_dir.to_path_buf()
}

pub fn copy_dir_contents(source: &Path, destination: &Path) -> io::Result<()> {
    for entry in WalkDir::new(source) {
        let entry = entry.map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;
        let path = entry.path();
        let rel = match path.strip_prefix(source) {
            Ok(r) if !r.as_os_str().is_empty() => r,
            _ => continue,
        };

        let target = destination.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        if entry.file_type().is_file() {
            fs::copy(path, &target)?;
        }
    }

    Ok(())
}

pub fn write_metadata_file(
    destination: &Path,
    install_target: InstallTarget,
    version: &str,
    notes: &str,
    archive_path: &Path,
) -> io::Result<()> {
    let mut file = File::create(destination.join("_mxbmm_meta.txt"))?;
    writeln!(file, "install_target={}", install_target.relative_path())?;
    writeln!(file, "version={}", version.trim())?;
    writeln!(file, "archive={}", archive_path.display())?;
    writeln!(file, "notes={}", notes.replace('\n', "\\n"))?;
    Ok(())
}

pub fn create_temp_extract_dir() -> io::Result<PathBuf> {
    let base = std::env::temp_dir().join("mxbmm_extracts");
    fs::create_dir_all(&base)?;

    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();

    for attempt in 0..1000_u32 {
        let dir = base.join(format!("extract-{}-{}-{}", pid, now_nanos, attempt));
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
            return Ok(dir);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Failed to create a unique extraction directory.",
    ))
}

pub fn create_fs_watcher(root: &Path) -> notify::Result<FsWatcherState> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;

    Ok(FsWatcherState {
        root: root.to_path_buf(),
        _watcher: watcher,
        rx,
    })
}
