use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use crossbeam_channel::Sender;

use crate::model::{Event, FindRequest, FindUpdate, FsEntry, FsEntryType};

const FIND_PROGRESS_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFindInput {
    pub query: String,
    pub glob: bool,
    pub hidden: bool,
    pub follow_symlinks: bool,
}

pub fn parse_find_input(input: &str, default_hidden: bool) -> Result<ParsedFindInput> {
    let mut tokens = input.split_whitespace();
    let query = tokens
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("find query cannot be empty"))?
        .to_string();

    let mut glob = false;
    let mut hidden = default_hidden;
    let mut follow_symlinks = false;

    for token in tokens {
        match token {
            "--glob" | "-g" => glob = true,
            "--name" => glob = false,
            "--hidden" | "-H" => hidden = true,
            "--no-hidden" => hidden = false,
            "--follow" | "-L" => follow_symlinks = true,
            "--no-follow" => follow_symlinks = false,
            _ => bail!("unknown find option '{token}'. Supported: --glob --hidden --follow"),
        }
    }

    Ok(ParsedFindInput {
        query,
        glob,
        hidden,
        follow_symlinks,
    })
}

pub fn is_fd_available() -> bool {
    ProcessCommand::new("fd")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

pub fn spawn_fd_search(request: FindRequest, event_tx: Sender<Event>) {
    thread::spawn(move || {
        if let Err(err) = run_fd_search(request.clone(), &event_tx) {
            let _ = event_tx.send(Event::Find(FindUpdate::Failed {
                id: request.id,
                panel_id: request.panel_id,
                query: request.query,
                error: err.to_string(),
            }));
        }
    });
}

fn run_fd_search(request: FindRequest, event_tx: &Sender<Event>) -> Result<()> {
    let mut command = ProcessCommand::new("fd");
    command
        .arg("--absolute-path")
        .arg("--color")
        .arg("never")
        .arg("--print0");
    if request.glob {
        command.arg("--glob");
    }
    if request.hidden {
        command.arg("--hidden");
    }
    if request.follow_symlinks {
        command.arg("--follow");
    }
    command
        .arg("--")
        .arg(request.query.as_str())
        .arg(request.root.as_os_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|err| anyhow::anyhow!("failed to start fd: {err}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to capture fd stdout"))?;
    let mut stderr = child.stderr.take();
    let mut reader = BufReader::new(stdout);
    let mut entries = Vec::new();
    let mut matches = 0usize;
    let mut last_progress = Instant::now()
        .checked_sub(FIND_PROGRESS_INTERVAL)
        .unwrap_or_else(Instant::now);

    loop {
        let mut raw = Vec::new();
        let read = reader.read_until(0, &mut raw)?;
        if read == 0 {
            break;
        }
        if raw.last() == Some(&0) {
            raw.pop();
        }
        if raw.is_empty() {
            continue;
        }

        let path = PathBuf::from(String::from_utf8_lossy(raw.as_slice()).to_string());
        entries.push(entry_from_path(path.as_path()));
        matches = matches.saturating_add(1);
        if last_progress.elapsed() >= FIND_PROGRESS_INTERVAL {
            let _ = event_tx.send(Event::Find(FindUpdate::Progress {
                id: request.id,
                panel_id: request.panel_id,
                query: request.query.clone(),
                matches,
            }));
            last_progress = Instant::now();
        }
    }

    let status = child
        .wait()
        .map_err(|err| anyhow::anyhow!("failed waiting for fd: {err}"))?;
    let mut stderr_text = String::new();
    if let Some(stderr_pipe) = stderr.as_mut() {
        let _ = stderr_pipe.read_to_string(&mut stderr_text);
    }
    if !status.success() {
        let stderr_text = stderr_text.trim();
        if stderr_text.is_empty() {
            bail!("fd exited with status {status}");
        }
        bail!("fd failed: {stderr_text}");
    }

    let _ = event_tx.send(Event::Find(FindUpdate::Done {
        id: request.id,
        panel_id: request.panel_id,
        query: request.query,
        root: request.root,
        glob: request.glob,
        hidden: request.hidden,
        follow_symlinks: request.follow_symlinks,
        entries,
    }));
    Ok(())
}

fn entry_from_path(path: &Path) -> FsEntry {
    let metadata = fs::symlink_metadata(path).ok();
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let entry_type = metadata
        .as_ref()
        .map(|meta| {
            let file_type = meta.file_type();
            if file_type.is_dir() {
                FsEntryType::Directory
            } else if file_type.is_symlink() {
                FsEntryType::Symlink
            } else if file_type.is_file() {
                FsEntryType::File
            } else {
                FsEntryType::Other
            }
        })
        .unwrap_or(FsEntryType::Other);
    let size_bytes = metadata
        .as_ref()
        .map(|meta| {
            if entry_type == FsEntryType::File {
                meta.len()
            } else {
                0
            }
        })
        .unwrap_or(0);
    let modified_at = metadata.as_ref().and_then(|meta| meta.modified().ok());
    let is_executable = metadata.as_ref().map(|meta| is_exec(meta)).unwrap_or(false);

    FsEntry {
        name: file_name.clone(),
        path: path.to_path_buf(),
        entry_type,
        size_bytes,
        modified_at,
        is_executable,
        is_hidden: file_name.starts_with('.'),
        is_virtual: false,
    }
}

#[cfg(unix)]
fn is_exec(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_exec(_: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::{ParsedFindInput, parse_find_input};

    #[test]
    fn parse_find_input_parses_query_and_flags() {
        let parsed = parse_find_input("main.rs --glob --hidden --follow", false).unwrap();
        assert_eq!(
            parsed,
            ParsedFindInput {
                query: "main.rs".to_string(),
                glob: true,
                hidden: true,
                follow_symlinks: true,
            }
        );
    }

    #[test]
    fn parse_find_input_uses_default_hidden() {
        let parsed = parse_find_input("Cargo.toml", true).unwrap();
        assert_eq!(parsed.query, "Cargo.toml");
        assert!(!parsed.glob);
        assert!(parsed.hidden);
        assert!(!parsed.follow_symlinks);
    }

    #[test]
    fn parse_find_input_rejects_unknown_flag() {
        let err = parse_find_input("foo --wat", false).unwrap_err();
        assert!(
            err.to_string().contains("unknown find option"),
            "unexpected err: {err}"
        );
    }
}
