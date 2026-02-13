#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use flate2::read::GzDecoder;
use ssh2::{FileStat, OpenFlags, OpenType, RenameFlags, Session, Sftp};
use tar::Archive as TarArchive;
use zip::ZipArchive;

use crate::fs::FsAdapter;
use crate::model::{
    ArchiveConnectionInfo, BackendSpec, FsEntry, FsEntryType, SftpAuth, SftpConnectionInfo,
    SortMode,
};

const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;
const SFTP_CONNECT_ATTEMPTS: usize = 3;

pub trait FsBackend: Send + Sync {
    fn backend_name(&self) -> &'static str;
    fn list_dir(&self, path: &Path, sort_mode: SortMode, show_hidden: bool)
    -> Result<Vec<FsEntry>>;
    fn stat_entry(&self, path: &Path) -> Result<FsEntry>;
    fn create_dir(&self, path: &Path) -> Result<()>;
    fn remove_path(&self, path: &Path) -> Result<()>;
    fn move_path(&self, source: &Path, destination: &Path) -> Result<PathBuf>;
    fn copy_path(&self, source: &Path, destination: &Path) -> Result<PathBuf>;
    fn normalize_existing_path(&self, operation: &'static str, path: &Path) -> Result<PathBuf>;
    fn normalize_new_path(&self, operation: &'static str, path: &Path) -> Result<PathBuf>;
    fn read_file(&self, path: &Path) -> Result<Vec<u8>>;
    fn write_file(&self, path: &Path, bytes: &[u8]) -> Result<()>;
}

pub fn backend_from_spec(spec: &BackendSpec) -> Arc<dyn FsBackend> {
    match spec {
        BackendSpec::Local => Arc::new(LocalFsBackend::default()),
        BackendSpec::Sftp(info) => Arc::new(SftpFsBackend::new(info.clone())),
        BackendSpec::Archive(info) => Arc::new(ArchiveFsBackend::new(info.clone())),
    }
}

#[derive(Default)]
pub struct LocalFsBackend {
    fs: FsAdapter,
}

impl FsBackend for LocalFsBackend {
    fn backend_name(&self) -> &'static str {
        "local"
    }

    fn list_dir(
        &self,
        path: &Path,
        sort_mode: SortMode,
        show_hidden: bool,
    ) -> Result<Vec<FsEntry>> {
        Ok(self.fs.list_dir(path, sort_mode, show_hidden)?)
    }

    fn stat_entry(&self, path: &Path) -> Result<FsEntry> {
        Ok(self.fs.stat_entry(path)?)
    }

    fn create_dir(&self, path: &Path) -> Result<()> {
        Ok(self.fs.create_dir(path)?)
    }

    fn remove_path(&self, path: &Path) -> Result<()> {
        Ok(self.fs.remove_path(path)?)
    }

    fn move_path(&self, source: &Path, destination: &Path) -> Result<PathBuf> {
        Ok(self.fs.move_path(source, destination)?)
    }

    fn copy_path(&self, source: &Path, destination: &Path) -> Result<PathBuf> {
        Ok(self.fs.copy_path(source, destination)?)
    }

    fn normalize_existing_path(&self, operation: &'static str, path: &Path) -> Result<PathBuf> {
        Ok(self.fs.normalize_existing_path(operation, path)?)
    }

    fn normalize_new_path(&self, operation: &'static str, path: &Path) -> Result<PathBuf> {
        Ok(self.fs.normalize_new_path(operation, path)?)
    }

    fn read_file(&self, path: &Path) -> Result<Vec<u8>> {
        let normalized = self.fs.normalize_existing_path("read", path)?;
        Ok(fs::read(normalized)?)
    }

    fn write_file(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        let normalized = self.fs.normalize_new_path("write", path)?;
        fs::write(normalized, bytes)?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct SftpFsBackend {
    conn: SftpConnectionInfo,
}

impl SftpFsBackend {
    pub fn new(conn: SftpConnectionInfo) -> Self {
        Self { conn }
    }

    fn connect(&self) -> Result<(Session, Sftp)> {
        let mut last_error: Option<anyhow::Error> = None;
        for attempt in 1..=SFTP_CONNECT_ATTEMPTS {
            match self.connect_once() {
                Ok(conn) => return Ok(conn),
                Err(err) => {
                    last_error = Some(err);
                    if attempt < SFTP_CONNECT_ATTEMPTS {
                        thread::sleep(Duration::from_millis((attempt as u64) * 120));
                    }
                }
            }
        }

        let err = last_error.unwrap_or_else(|| anyhow::anyhow!("unknown sftp error"));
        Err(anyhow::anyhow!(
            "sftp connect failed [{}]: {err}",
            classify_sftp_error(&err)
        ))
    }

    fn connect_once(&self) -> Result<(Session, Sftp)> {
        let endpoint = format!("{}:{}", self.conn.host, self.conn.port);
        let tcp = TcpStream::connect(endpoint.as_str())?;
        tcp.set_read_timeout(Some(Duration::from_secs(30)))?;
        tcp.set_write_timeout(Some(Duration::from_secs(30)))?;

        let mut session = Session::new()?;
        session.set_tcp_stream(tcp);
        session.handshake()?;

        match &self.conn.auth {
            SftpAuth::Agent => session.userauth_agent(self.conn.user.as_str())?,
            SftpAuth::Password(password) => {
                session.userauth_password(self.conn.user.as_str(), password.as_str())?
            }
            SftpAuth::KeyFile { path, passphrase } => session.userauth_pubkey_file(
                self.conn.user.as_str(),
                None,
                path.as_path(),
                passphrase.as_deref(),
            )?,
        }

        if !session.authenticated() {
            bail!("sftp auth failed for {}", self.conn.user);
        }
        let sftp = session.sftp()?;
        Ok((session, sftp))
    }

    fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.conn.root_path.join(path)
        }
    }
}

impl FsBackend for SftpFsBackend {
    fn backend_name(&self) -> &'static str {
        "sftp"
    }

    fn list_dir(
        &self,
        path: &Path,
        sort_mode: SortMode,
        show_hidden: bool,
    ) -> Result<Vec<FsEntry>> {
        let (_session, sftp) = self.connect()?;
        let dir = self.normalize_existing_path("list_dir", path)?;
        let mut entries = Vec::new();
        for (entry_path, stat) in sftp.readdir(&dir)? {
            let Some(name) = entry_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
            else {
                continue;
            };
            if name == "." || name == ".." {
                continue;
            }
            let is_hidden = name.starts_with('.');
            if !show_hidden && is_hidden {
                continue;
            }
            entries.push(FsEntry {
                name,
                path: entry_path.clone(),
                entry_type: entry_type_from_stat(&stat),
                size_bytes: stat.size.unwrap_or(0),
                modified_at: modified_from_stat(&stat),
                is_executable: is_exec_from_stat(&stat),
                is_hidden,
                is_virtual: false,
            });
        }

        sort_entries(entries.as_mut_slice(), sort_mode);
        if let Some(parent) = dir.parent() {
            if parent != dir {
                entries.insert(0, parent_link(parent.to_path_buf()));
            }
        }
        Ok(entries)
    }

    fn stat_entry(&self, path: &Path) -> Result<FsEntry> {
        let (_session, sftp) = self.connect()?;
        let normalized = self.normalize_existing_path("stat", path)?;
        let stat = sftp.stat(normalized.as_path())?;
        let name = normalized
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| normalized.display().to_string());
        Ok(FsEntry {
            name: name.clone(),
            path: normalized,
            entry_type: entry_type_from_stat(&stat),
            size_bytes: stat.size.unwrap_or(0),
            modified_at: modified_from_stat(&stat),
            is_executable: is_exec_from_stat(&stat),
            is_hidden: name.starts_with('.'),
            is_virtual: false,
        })
    }

    fn create_dir(&self, path: &Path) -> Result<()> {
        let (_session, sftp) = self.connect()?;
        let normalized = self.normalize_new_path("mkdir", path)?;
        sftp.mkdir(normalized.as_path(), 0o755)?;
        Ok(())
    }

    fn remove_path(&self, path: &Path) -> Result<()> {
        let (_session, sftp) = self.connect()?;
        let normalized = self.normalize_existing_path("remove", path)?;
        remove_remote_path_recursive(&sftp, normalized.as_path())
    }

    fn move_path(&self, source: &Path, destination: &Path) -> Result<PathBuf> {
        let (_session, sftp) = self.connect()?;
        let source_path = self.normalize_existing_path("move", source)?;
        let destination_path = self.normalize_new_path("move", destination)?;
        sftp.rename(
            source_path.as_path(),
            destination_path.as_path(),
            Some(RenameFlags::empty()),
        )?;
        Ok(destination_path)
    }

    fn copy_path(&self, source: &Path, destination: &Path) -> Result<PathBuf> {
        let (_session, sftp) = self.connect()?;
        let source_path = self.normalize_existing_path("copy", source)?;
        let destination_path = self.normalize_new_path("copy", destination)?;
        copy_remote_path_recursive(&sftp, source_path.as_path(), destination_path.as_path())?;
        Ok(destination_path)
    }

    fn normalize_existing_path(&self, _operation: &'static str, path: &Path) -> Result<PathBuf> {
        let (_session, sftp) = self.connect()?;
        let resolved = self.resolve_path(path);
        match sftp.realpath(resolved.as_path()) {
            Ok(path) => Ok(path),
            Err(_) => Ok(resolved),
        }
    }

    fn normalize_new_path(&self, _operation: &'static str, path: &Path) -> Result<PathBuf> {
        Ok(self.resolve_path(path))
    }

    fn read_file(&self, path: &Path) -> Result<Vec<u8>> {
        let (_session, sftp) = self.connect()?;
        let normalized = self.normalize_existing_path("read", path)?;
        let mut file = sftp.open(normalized.as_path())?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        Ok(buffer)
    }

    fn write_file(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        let (_session, sftp) = self.connect()?;
        let normalized = self.normalize_new_path("write", path)?;
        let mut file = sftp.open_mode(
            normalized.as_path(),
            OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
            0o644,
            OpenType::File,
        )?;
        file.write_all(bytes)?;
        Ok(())
    }
}

pub struct ArchiveFsBackend {
    conn: ArchiveConnectionInfo,
}

impl ArchiveFsBackend {
    pub fn new(conn: ArchiveConnectionInfo) -> Self {
        Self { conn }
    }

    fn index(&self) -> Result<ArchiveIndex> {
        build_archive_index(self.conn.archive_path.as_path())
    }
}

impl FsBackend for ArchiveFsBackend {
    fn backend_name(&self) -> &'static str {
        "archive"
    }

    fn list_dir(
        &self,
        path: &Path,
        sort_mode: SortMode,
        show_hidden: bool,
    ) -> Result<Vec<FsEntry>> {
        let normalized = self.normalize_existing_path("list_dir", path)?;
        let index = self.index()?;
        let entry = index.entries.get(&normalized).ok_or_else(|| {
            anyhow::anyhow!("path not found in archive: {}", normalized.display())
        })?;
        if entry.entry_type != FsEntryType::Directory {
            bail!(
                "path is not a directory in archive: {}",
                normalized.display()
            );
        }

        let mut entries = Vec::new();
        if let Some(children) = index.children.get(&normalized) {
            for child_path in children {
                if let Some(child) = index.entries.get(child_path) {
                    if !show_hidden && child.is_hidden {
                        continue;
                    }
                    entries.push(child.clone());
                }
            }
        }

        sort_entries(entries.as_mut_slice(), sort_mode);
        if normalized != Path::new("/") {
            if let Some(parent) = normalized.parent() {
                entries.insert(0, parent_link(parent.to_path_buf()));
            } else {
                entries.insert(0, parent_link(PathBuf::from("/")));
            }
        }
        entries.insert(0, archive_exit_link());
        Ok(entries)
    }

    fn stat_entry(&self, path: &Path) -> Result<FsEntry> {
        let normalized = self.normalize_existing_path("stat", path)?;
        let index = self.index()?;
        index
            .entries
            .get(&normalized)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("path not found in archive: {}", normalized.display()))
    }

    fn create_dir(&self, path: &Path) -> Result<()> {
        bail!(
            "archive backend is read-only (mkdir is unsupported): {}",
            path.display()
        )
    }

    fn remove_path(&self, path: &Path) -> Result<()> {
        bail!(
            "archive backend is read-only (remove is unsupported): {}",
            path.display()
        )
    }

    fn move_path(&self, source: &Path, destination: &Path) -> Result<PathBuf> {
        bail!(
            "archive backend is read-only (move is unsupported): {} -> {}",
            source.display(),
            destination.display()
        )
    }

    fn copy_path(&self, source: &Path, destination: &Path) -> Result<PathBuf> {
        bail!(
            "archive backend is read-only (copy inside archive is unsupported): {} -> {}",
            source.display(),
            destination.display()
        )
    }

    fn normalize_existing_path(&self, _operation: &'static str, path: &Path) -> Result<PathBuf> {
        let normalized = normalize_archive_virtual_path(path);
        if normalized == Path::new("/") {
            return Ok(normalized);
        }
        let index = self.index()?;
        if index.entries.contains_key(&normalized) {
            Ok(normalized)
        } else {
            bail!("path not found in archive: {}", normalized.display())
        }
    }

    fn normalize_new_path(&self, _operation: &'static str, path: &Path) -> Result<PathBuf> {
        Ok(normalize_archive_virtual_path(path))
    }

    fn read_file(&self, path: &Path) -> Result<Vec<u8>> {
        let normalized = self.normalize_existing_path("read", path)?;
        let index = self.index()?;
        let entry = index.entries.get(&normalized).ok_or_else(|| {
            anyhow::anyhow!("path not found in archive: {}", normalized.display())
        })?;
        if entry.entry_type == FsEntryType::Directory {
            bail!(
                "cannot read directory from archive: {}",
                normalized.display()
            );
        }
        read_archive_member(
            self.conn.archive_path.as_path(),
            normalized.as_path(),
            detect_archive_format(self.conn.archive_path.as_path())
                .ok_or_else(|| anyhow::anyhow!("unsupported archive format"))?,
        )
    }

    fn write_file(&self, path: &Path, _bytes: &[u8]) -> Result<()> {
        bail!(
            "archive backend is read-only (write is unsupported): {}",
            path.display()
        )
    }
}

#[derive(Clone, Copy)]
enum ArchiveFormat {
    Zip,
    Tar,
    TarGz,
}

struct ArchiveIndex {
    entries: HashMap<PathBuf, FsEntry>,
    children: HashMap<PathBuf, Vec<PathBuf>>,
}

impl ArchiveIndex {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            children: HashMap::new(),
        }
    }

    fn insert_directory(&mut self, path: PathBuf) {
        if path == Path::new("/") {
            self.entries.entry(path.clone()).or_insert(FsEntry {
                name: "/".to_string(),
                path: path.clone(),
                entry_type: FsEntryType::Directory,
                size_bytes: 0,
                modified_at: None,
                is_executable: false,
                is_hidden: false,
                is_virtual: false,
            });
            return;
        }

        self.ensure_parent_chain(path.as_path());
        if !self.entries.contains_key(&path) {
            let name = path
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string());
            self.entries.insert(
                path.clone(),
                FsEntry {
                    name: name.clone(),
                    path: path.clone(),
                    entry_type: FsEntryType::Directory,
                    size_bytes: 0,
                    modified_at: None,
                    is_executable: false,
                    is_hidden: name.starts_with('.'),
                    is_virtual: false,
                },
            );
        }
        self.register_child(path.as_path());
    }

    fn insert_file(&mut self, path: PathBuf, size_bytes: u64, entry_type: FsEntryType) {
        self.ensure_parent_chain(path.as_path());
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        self.entries.insert(
            path.clone(),
            FsEntry {
                name: name.clone(),
                path: path.clone(),
                entry_type,
                size_bytes,
                modified_at: None,
                is_executable: false,
                is_hidden: name.starts_with('.'),
                is_virtual: false,
            },
        );
        self.register_child(path.as_path());
    }

    fn ensure_parent_chain(&mut self, path: &Path) {
        self.entries.entry(PathBuf::from("/")).or_insert(FsEntry {
            name: "/".to_string(),
            path: PathBuf::from("/"),
            entry_type: FsEntryType::Directory,
            size_bytes: 0,
            modified_at: None,
            is_executable: false,
            is_hidden: false,
            is_virtual: false,
        });

        let mut cursor = PathBuf::from("/");
        for component in path.components() {
            if let std::path::Component::Normal(value) = component {
                cursor.push(value);
                if cursor == path {
                    break;
                }
                if !self.entries.contains_key(&cursor) {
                    let name = cursor
                        .file_name()
                        .map(|v| v.to_string_lossy().to_string())
                        .unwrap_or_else(|| cursor.display().to_string());
                    self.entries.insert(
                        cursor.clone(),
                        FsEntry {
                            name: name.clone(),
                            path: cursor.clone(),
                            entry_type: FsEntryType::Directory,
                            size_bytes: 0,
                            modified_at: None,
                            is_executable: false,
                            is_hidden: name.starts_with('.'),
                            is_virtual: false,
                        },
                    );
                }
                self.register_child(cursor.as_path());
            }
        }
    }

    fn register_child(&mut self, path: &Path) {
        if path == Path::new("/") {
            return;
        }
        let parent = path
            .parent()
            .unwrap_or_else(|| Path::new("/"))
            .to_path_buf();
        let list = self.children.entry(parent).or_default();
        let target = path.to_path_buf();
        if !list.contains(&target) {
            list.push(target);
        }
    }
}

pub fn is_archive_file_path(path: &Path) -> bool {
    detect_archive_format(path).is_some()
}

fn detect_archive_format(path: &Path) -> Option<ArchiveFormat> {
    let value = path.to_string_lossy().to_ascii_lowercase();
    if value.ends_with(".zip") {
        Some(ArchiveFormat::Zip)
    } else if value.ends_with(".tar.gz") || value.ends_with(".tgz") {
        Some(ArchiveFormat::TarGz)
    } else if value.ends_with(".tar") {
        Some(ArchiveFormat::Tar)
    } else {
        None
    }
}

fn build_archive_index(archive_path: &Path) -> Result<ArchiveIndex> {
    let format = detect_archive_format(archive_path)
        .ok_or_else(|| anyhow::anyhow!("unsupported archive format: {}", archive_path.display()))?;
    let mut index = ArchiveIndex::new();
    index.insert_directory(PathBuf::from("/"));

    match format {
        ArchiveFormat::Zip => build_zip_index(archive_path, &mut index)?,
        ArchiveFormat::Tar => build_tar_index(archive_path, &mut index, false)?,
        ArchiveFormat::TarGz => build_tar_index(archive_path, &mut index, true)?,
    }
    Ok(index)
}

fn build_zip_index(archive_path: &Path, index: &mut ArchiveIndex) -> Result<()> {
    let file = fs::File::open(archive_path)?;
    let mut archive = ZipArchive::new(file)?;
    for idx in 0..archive.len() {
        let file = archive.by_index(idx)?;
        let Some(path) = archive_member_to_virtual_path(file.name()) else {
            continue;
        };
        if file.is_dir() || file.name().ends_with('/') {
            index.insert_directory(path);
        } else {
            index.insert_file(path, file.size(), FsEntryType::File);
        }
    }
    Ok(())
}

fn build_tar_index(archive_path: &Path, index: &mut ArchiveIndex, compressed: bool) -> Result<()> {
    if compressed {
        let file = fs::File::open(archive_path)?;
        let decoder = GzDecoder::new(file);
        let mut archive = TarArchive::new(decoder);
        for item in archive.entries()? {
            let entry = item?;
            register_tar_entry(&entry, index)?;
        }
    } else {
        let file = fs::File::open(archive_path)?;
        let mut archive = TarArchive::new(file);
        for item in archive.entries()? {
            let entry = item?;
            register_tar_entry(&entry, index)?;
        }
    }
    Ok(())
}

fn register_tar_entry<R: Read>(entry: &tar::Entry<'_, R>, index: &mut ArchiveIndex) -> Result<()> {
    let raw = entry.path()?;
    let Some(path) = archive_member_to_virtual_path(raw.to_string_lossy().as_ref()) else {
        return Ok(());
    };
    let entry_type = entry.header().entry_type();
    if entry_type.is_dir() {
        index.insert_directory(path);
    } else if entry_type.is_symlink() {
        index.insert_file(path, 0, FsEntryType::Symlink);
    } else {
        index.insert_file(path, entry.size(), FsEntryType::File);
    }
    Ok(())
}

fn read_archive_member(
    archive_path: &Path,
    member: &Path,
    format: ArchiveFormat,
) -> Result<Vec<u8>> {
    match format {
        ArchiveFormat::Zip => read_zip_member(archive_path, member),
        ArchiveFormat::Tar => read_tar_member(archive_path, member, false),
        ArchiveFormat::TarGz => read_tar_member(archive_path, member, true),
    }
}

fn read_zip_member(archive_path: &Path, member: &Path) -> Result<Vec<u8>> {
    let file = fs::File::open(archive_path)?;
    let mut archive = ZipArchive::new(file)?;
    for idx in 0..archive.len() {
        let mut file = archive.by_index(idx)?;
        let Some(path) = archive_member_to_virtual_path(file.name()) else {
            continue;
        };
        if path != member {
            continue;
        }
        if file.is_dir() || file.name().ends_with('/') {
            bail!("archive member is a directory: {}", member.display());
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        return Ok(bytes);
    }
    bail!("archive member not found: {}", member.display())
}

fn read_tar_member(archive_path: &Path, member: &Path, compressed: bool) -> Result<Vec<u8>> {
    if compressed {
        let file = fs::File::open(archive_path)?;
        let decoder = GzDecoder::new(file);
        let mut archive = TarArchive::new(decoder);
        for item in archive.entries()? {
            let mut entry = item?;
            let Some(path) =
                archive_member_to_virtual_path(entry.path()?.to_string_lossy().as_ref())
            else {
                continue;
            };
            if path != member {
                continue;
            }
            if entry.header().entry_type().is_dir() {
                bail!("archive member is a directory: {}", member.display());
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            return Ok(bytes);
        }
    } else {
        let file = fs::File::open(archive_path)?;
        let mut archive = TarArchive::new(file);
        for item in archive.entries()? {
            let mut entry = item?;
            let Some(path) =
                archive_member_to_virtual_path(entry.path()?.to_string_lossy().as_ref())
            else {
                continue;
            };
            if path != member {
                continue;
            }
            if entry.header().entry_type().is_dir() {
                bail!("archive member is a directory: {}", member.display());
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            return Ok(bytes);
        }
    }
    bail!("archive member not found: {}", member.display())
}

fn archive_member_to_virtual_path(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = trimmed.replace('\\', "/");
    let stripped = normalized.trim_start_matches('/').trim_end_matches('/');
    if stripped.is_empty() {
        return Some(PathBuf::from("/"));
    }
    let joined = PathBuf::from("/").join(stripped);
    Some(normalize_archive_virtual_path(joined.as_path()))
}

fn normalize_archive_virtual_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::from("/");
    for component in path.components() {
        match component {
            std::path::Component::RootDir | std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if out != Path::new("/") {
                    out.pop();
                    if out.as_os_str().is_empty() {
                        out = PathBuf::from("/");
                    }
                }
            }
            std::path::Component::Normal(value) => out.push(value),
            std::path::Component::Prefix(_) => {}
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        out
    }
}

fn entry_type_from_stat(stat: &FileStat) -> FsEntryType {
    let mode = stat.perm.unwrap_or(0);
    match mode & S_IFMT {
        S_IFDIR => FsEntryType::Directory,
        S_IFLNK => FsEntryType::Symlink,
        _ => FsEntryType::File,
    }
}

fn is_exec_from_stat(stat: &FileStat) -> bool {
    stat.perm.unwrap_or(0) & 0o111 != 0
}

fn modified_from_stat(stat: &FileStat) -> Option<SystemTime> {
    stat.mtime
        .map(|secs| UNIX_EPOCH + Duration::from_secs(secs))
}

fn parent_link(parent: PathBuf) -> FsEntry {
    FsEntry {
        name: "..".to_string(),
        path: parent,
        entry_type: FsEntryType::Directory,
        size_bytes: 0,
        modified_at: None,
        is_executable: false,
        is_hidden: false,
        is_virtual: true,
    }
}

fn archive_exit_link() -> FsEntry {
    FsEntry {
        name: ":".to_string(),
        path: PathBuf::from("/"),
        entry_type: FsEntryType::Directory,
        size_bytes: 0,
        modified_at: None,
        is_executable: false,
        is_hidden: false,
        is_virtual: true,
    }
}

fn sort_entries(entries: &mut [FsEntry], sort_mode: SortMode) {
    entries.sort_by(|left, right| {
        let type_cmp = entry_group(left).cmp(&entry_group(right));
        if type_cmp != std::cmp::Ordering::Equal {
            return type_cmp;
        }

        match sort_mode {
            SortMode::Name => cmp_name(left, right),
            SortMode::Size => left
                .size_bytes
                .cmp(&right.size_bytes)
                .reverse()
                .then_with(|| cmp_name(left, right)),
            SortMode::ModifiedAt => {
                cmp_modified_at(left, right).then_with(|| cmp_name(left, right))
            }
        }
    });
}

fn entry_group(entry: &FsEntry) -> u8 {
    match entry.entry_type {
        FsEntryType::Directory => 0,
        _ => 1,
    }
}

fn cmp_name(left: &FsEntry, right: &FsEntry) -> std::cmp::Ordering {
    left.name.to_lowercase().cmp(&right.name.to_lowercase())
}

fn cmp_modified_at(left: &FsEntry, right: &FsEntry) -> std::cmp::Ordering {
    right
        .modified_at
        .unwrap_or(UNIX_EPOCH)
        .cmp(&left.modified_at.unwrap_or(UNIX_EPOCH))
}

fn remove_remote_path_recursive(sftp: &Sftp, path: &Path) -> Result<()> {
    let stat = sftp.stat(path)?;
    if entry_type_from_stat(&stat) == FsEntryType::Directory {
        for (child_path, _) in sftp.readdir(path)? {
            let Some(name) = child_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
            else {
                continue;
            };
            if name == "." || name == ".." {
                continue;
            }
            remove_remote_path_recursive(sftp, child_path.as_path())?;
        }
        sftp.rmdir(path)?;
    } else {
        sftp.unlink(path)?;
    }
    Ok(())
}

fn classify_sftp_error(err: &anyhow::Error) -> &'static str {
    let lower = err.to_string().to_ascii_lowercase();
    if lower.contains("auth")
        || lower.contains("password")
        || lower.contains("publickey")
        || lower.contains("identity")
        || lower.contains("identities")
        || lower.contains("ssh agent")
        || lower.contains("agent")
    {
        "auth"
    } else if lower.contains("permission denied") {
        "perm"
    } else if lower.contains("not found") || lower.contains("no such file") {
        "path"
    } else if lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("connection")
        || lower.contains("refused")
        || lower.contains("network")
    {
        "network"
    } else {
        "unknown"
    }
}

fn copy_remote_path_recursive(sftp: &Sftp, source: &Path, destination: &Path) -> Result<()> {
    let stat = sftp.stat(source)?;
    if entry_type_from_stat(&stat) == FsEntryType::Directory {
        let _ = sftp.mkdir(destination, 0o755);
        for (child_path, _) in sftp.readdir(source)? {
            let Some(name) = child_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
            else {
                continue;
            };
            if name == "." || name == ".." {
                continue;
            }
            let target = destination.join(name);
            copy_remote_path_recursive(sftp, child_path.as_path(), target.as_path())?;
        }
        return Ok(());
    }

    let mut src_file = sftp.open(source)?;
    let mut dst_file = sftp.open_mode(
        destination,
        OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
        0o644,
        OpenType::File,
    )?;
    let mut buffer = vec![0_u8; 32 * 1024];
    loop {
        let read = src_file.read(buffer.as_mut_slice())?;
        if read == 0 {
            break;
        }
        dst_file.write_all(&buffer[..read])?;
    }
    Ok(())
}
