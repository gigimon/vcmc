#![allow(dead_code)]

use std::cmp::Ordering;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use walkdir::WalkDir;

use crate::errors::{AppError, AppResult};
use crate::model::{FsEntry, FsEntryType, SortMode};

#[derive(Debug, Default, Clone)]
pub struct FsAdapter;

impl FsAdapter {
    pub fn list_dir(
        &self,
        path: &Path,
        sort_mode: SortMode,
        show_hidden: bool,
    ) -> AppResult<Vec<FsEntry>> {
        let dir_path = self.normalize_existing_path("list_dir", path)?;
        let dir_iter = fs::read_dir(&dir_path)
            .map_err(|err| AppError::from_io("list_dir", dir_path.clone(), err))?;

        let mut entries = Vec::new();
        for entry_result in dir_iter {
            let entry =
                entry_result.map_err(|err| AppError::from_io("list_dir", dir_path.clone(), err))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|err| AppError::from_io("list_dir", path.clone(), err))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let is_hidden = name.starts_with('.');
            if !show_hidden && is_hidden {
                continue;
            }

            let entry_type = map_entry_type(&metadata);
            let size_bytes = metadata.len();
            let modified_at = metadata.modified().ok();
            entries.push(FsEntry {
                name,
                path,
                entry_type,
                size_bytes,
                modified_at,
                is_hidden,
                is_virtual: false,
            });
        }

        sort_entries(&mut entries, sort_mode);
        if let Some(parent_path) = dir_path.parent() {
            entries.insert(0, parent_link(parent_path.to_path_buf()));
        }
        Ok(entries)
    }

    pub fn stat_entry(&self, path: &Path) -> AppResult<FsEntry> {
        let normalized = self.normalize_existing_path("stat", path)?;
        let metadata = fs::symlink_metadata(&normalized)
            .map_err(|err| AppError::from_io("stat", normalized.clone(), err))?;
        let name = normalized
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| normalized.display().to_string());
        let is_hidden = name.starts_with('.');

        Ok(FsEntry {
            name,
            path: normalized,
            entry_type: map_entry_type(&metadata),
            size_bytes: metadata.len(),
            modified_at: metadata.modified().ok(),
            is_hidden,
            is_virtual: false,
        })
    }

    pub fn create_dir(&self, path: &Path) -> AppResult<()> {
        let normalized = self.normalize_new_path("mkdir", path)?;
        fs::create_dir(&normalized).map_err(|err| AppError::from_io("mkdir", normalized, err))?;
        Ok(())
    }

    pub fn remove_path(&self, path: &Path) -> AppResult<()> {
        let normalized = self.normalize_existing_path("remove", path)?;
        let metadata = fs::symlink_metadata(&normalized)
            .map_err(|err| AppError::from_io("remove", normalized.clone(), err))?;

        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            fs::remove_dir_all(&normalized)
                .map_err(|err| AppError::from_io("remove", normalized, err))?;
        } else {
            fs::remove_file(&normalized)
                .map_err(|err| AppError::from_io("remove", normalized, err))?;
        }

        Ok(())
    }

    pub fn move_path(&self, source: &Path, destination: &Path) -> AppResult<PathBuf> {
        let source_path = self.normalize_existing_path("move", source)?;
        let destination_path = self.resolve_destination_path("move", &source_path, destination)?;

        if source_path == destination_path {
            return Err(AppError::invalid_path(
                "move",
                destination_path,
                "source and destination are the same",
            ));
        }

        match fs::rename(&source_path, &destination_path) {
            Ok(()) => Ok(destination_path),
            Err(err) if err.kind() == std::io::ErrorKind::CrossesDevices => {
                self.copy_path(&source_path, &destination_path)?;
                self.remove_path(&source_path)?;
                Ok(destination_path)
            }
            Err(err) => Err(AppError::from_io("move", source_path, err)),
        }
    }

    pub fn copy_path(&self, source: &Path, destination: &Path) -> AppResult<PathBuf> {
        let source_path = self.normalize_existing_path("copy", source)?;
        let destination_path = self.resolve_destination_path("copy", &source_path, destination)?;

        if source_path == destination_path {
            return Err(AppError::invalid_path(
                "copy",
                destination_path,
                "source and destination are the same",
            ));
        }

        let metadata = fs::symlink_metadata(&source_path)
            .map_err(|err| AppError::from_io("copy", source_path.clone(), err))?;

        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            copy_directory_recursive(&source_path, &destination_path)?;
        } else {
            copy_regular_or_symlink_file(&source_path, &destination_path)?;
        }

        Ok(destination_path)
    }

    pub fn normalize_existing_path(
        &self,
        operation: &'static str,
        path: &Path,
    ) -> AppResult<PathBuf> {
        let absolute = self.absolute_path(operation, path)?;
        fs::canonicalize(&absolute).map_err(|err| AppError::from_io(operation, absolute, err))
    }

    pub fn normalize_new_path(&self, operation: &'static str, path: &Path) -> AppResult<PathBuf> {
        let absolute = self.absolute_path(operation, path)?;
        let name = absolute.file_name().ok_or_else(|| {
            AppError::invalid_path(
                operation,
                absolute.clone(),
                "target path must include a file or directory name",
            )
        })?;

        let parent = absolute.parent().ok_or_else(|| {
            AppError::invalid_path(
                operation,
                absolute.clone(),
                "target path has no parent directory",
            )
        })?;
        let normalized_parent = fs::canonicalize(parent)
            .map_err(|err| AppError::from_io(operation, parent.to_path_buf(), err))?;

        Ok(normalized_parent.join(name))
    }

    fn resolve_destination_path(
        &self,
        operation: &'static str,
        source: &Path,
        destination: &Path,
    ) -> AppResult<PathBuf> {
        let mut destination_path = self.absolute_path(operation, destination)?;

        if destination_path
            .try_exists()
            .map_err(|err| AppError::from_io(operation, destination_path.clone(), err))?
        {
            let metadata = fs::symlink_metadata(&destination_path)
                .map_err(|err| AppError::from_io(operation, destination_path.clone(), err))?;
            if metadata.file_type().is_dir() {
                let source_name = source.file_name().ok_or_else(|| {
                    AppError::invalid_path(
                        operation,
                        source.to_path_buf(),
                        "source path has no terminal component",
                    )
                })?;
                destination_path = destination_path.join(source_name);
            }
        }

        self.normalize_new_path(operation, &destination_path)
    }

    fn absolute_path(&self, operation: &'static str, path: &Path) -> AppResult<PathBuf> {
        if path.is_absolute() {
            return Ok(path.to_path_buf());
        }

        let cwd = env::current_dir()
            .map_err(|err| AppError::from_io(operation, PathBuf::from("."), err))?;
        Ok(cwd.join(path))
    }
}

fn parent_link(parent: PathBuf) -> FsEntry {
    FsEntry {
        name: "..".to_string(),
        path: parent,
        entry_type: FsEntryType::Directory,
        size_bytes: 0,
        modified_at: None,
        is_hidden: false,
        is_virtual: true,
    }
}

fn map_entry_type(metadata: &fs::Metadata) -> FsEntryType {
    let file_type = metadata.file_type();
    if file_type.is_dir() {
        FsEntryType::Directory
    } else if file_type.is_file() {
        FsEntryType::File
    } else if file_type.is_symlink() {
        FsEntryType::Symlink
    } else {
        FsEntryType::Other
    }
}

fn sort_entries(entries: &mut [FsEntry], sort_mode: SortMode) {
    entries.sort_by(|left, right| {
        let type_cmp = entry_group(left).cmp(&entry_group(right));
        if type_cmp != Ordering::Equal {
            return type_cmp;
        }

        let mode_cmp = match sort_mode {
            SortMode::Name => cmp_name(left, right),
            SortMode::Size => left
                .size_bytes
                .cmp(&right.size_bytes)
                .reverse()
                .then_with(|| cmp_name(left, right)),
            SortMode::ModifiedAt => {
                cmp_modified_at(left, right).then_with(|| cmp_name(left, right))
            }
        };

        mode_cmp
    });
}

fn entry_group(entry: &FsEntry) -> u8 {
    match entry.entry_type {
        FsEntryType::Directory => 0,
        _ => 1,
    }
}

fn cmp_name(left: &FsEntry, right: &FsEntry) -> Ordering {
    left.name.to_lowercase().cmp(&right.name.to_lowercase())
}

fn cmp_modified_at(left: &FsEntry, right: &FsEntry) -> Ordering {
    let left_ts = left.modified_at.unwrap_or(SystemTime::UNIX_EPOCH);
    let right_ts = right.modified_at.unwrap_or(SystemTime::UNIX_EPOCH);
    right_ts.cmp(&left_ts)
}

fn copy_directory_recursive(source: &Path, destination: &Path) -> AppResult<()> {
    fs::create_dir_all(destination)
        .map_err(|err| AppError::from_io("copy", destination.to_path_buf(), err))?;

    for entry in WalkDir::new(source).follow_links(false).min_depth(1) {
        let entry = entry.map_err(|err| {
            let path = err.path().unwrap_or(source).to_path_buf();
            AppError::invalid_path("copy", path, err.to_string())
        })?;

        let relative = entry.path().strip_prefix(source).map_err(|err| {
            AppError::invalid_path("copy", entry.path().to_path_buf(), err.to_string())
        })?;
        let target = destination.join(relative);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|err| AppError::from_io("copy", target, err))?;
            continue;
        }

        copy_regular_or_symlink_file(entry.path(), &target)?;
    }

    Ok(())
}

fn copy_regular_or_symlink_file(source: &Path, destination: &Path) -> AppResult<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| AppError::from_io("copy", parent.to_path_buf(), err))?;
    }

    let metadata = fs::symlink_metadata(source)
        .map_err(|err| AppError::from_io("copy", source.to_path_buf(), err))?;
    if metadata.file_type().is_symlink() {
        let link_target = fs::read_link(source)
            .map_err(|err| AppError::from_io("copy", source.to_path_buf(), err))?;
        copy_symlink(&link_target, destination)?;
        return Ok(());
    }

    fs::copy(source, destination)
        .map_err(|err| AppError::from_io("copy", destination.to_path_buf(), err))?;
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(link_target: &Path, destination: &Path) -> AppResult<()> {
    use std::os::unix::fs as unix_fs;

    if destination
        .try_exists()
        .map_err(|err| AppError::from_io("copy", destination.to_path_buf(), err))?
    {
        fs::remove_file(destination)
            .map_err(|err| AppError::from_io("copy", destination.to_path_buf(), err))?;
    }
    unix_fs::symlink(link_target, destination)
        .map_err(|err| AppError::from_io("copy", destination.to_path_buf(), err))?;
    Ok(())
}
