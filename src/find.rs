use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use crossbeam_channel::Sender;

use crate::model::{Event, FindRequest, FindUpdate, FsEntry, FsEntryType};

const FIND_PROGRESS_INTERVAL: Duration = Duration::from_millis(150);
const CONTENT_SNIPPET_MAX_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFindInput {
    pub query: String,
    pub glob: bool,
    pub hidden: bool,
    pub follow_symlinks: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedContentSearchInput {
    pub pattern: String,
    pub glob_pattern: Option<String>,
    pub hidden: bool,
    pub case_sensitive: bool,
}

#[derive(Clone)]
struct FindControl {
    cancel: Arc<AtomicBool>,
    pid: u32,
}

enum FindRunOutcome {
    Done(Vec<FsEntry>),
    Canceled,
}

struct FindControlGuard {
    id: u64,
}

impl Drop for FindControlGuard {
    fn drop(&mut self) {
        unregister_find_control(self.id);
    }
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

pub fn parse_content_search_input(
    input: &str,
    default_hidden: bool,
    default_case_sensitive: bool,
) -> Result<ParsedContentSearchInput> {
    let mut tokens = input.split_whitespace();
    let pattern = tokens
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("search pattern cannot be empty"))?
        .to_string();

    let mut glob_pattern = None;
    let mut hidden = default_hidden;
    let mut case_sensitive = default_case_sensitive;
    while let Some(token) = tokens.next() {
        match token {
            "--hidden" | "-H" => hidden = true,
            "--no-hidden" => hidden = false,
            "--case-sensitive" | "-s" => case_sensitive = true,
            "--ignore-case" | "-i" => case_sensitive = false,
            "--glob" => {
                let value = tokens.next().ok_or_else(|| {
                    anyhow::anyhow!("--glob requires pattern value (example: --glob '*.rs')")
                })?;
                if value.trim().is_empty() {
                    bail!("--glob pattern cannot be empty");
                }
                glob_pattern = Some(value.to_string());
            }
            value if value.starts_with("--glob=") => {
                let Some((_, glob)) = value.split_once('=') else {
                    bail!("invalid --glob option");
                };
                if glob.trim().is_empty() {
                    bail!("--glob pattern cannot be empty");
                }
                glob_pattern = Some(glob.to_string());
            }
            _ => {
                bail!(
                    "unknown content-search option '{token}'. Supported: --glob --hidden --case-sensitive --ignore-case"
                )
            }
        }
    }

    Ok(ParsedContentSearchInput {
        pattern,
        glob_pattern,
        hidden,
        case_sensitive,
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

pub fn is_rg_available() -> bool {
    ProcessCommand::new("rg")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

pub fn cancel_running_find(id: u64) -> bool {
    let control = {
        let map = find_controls().lock();
        let Ok(map) = map else {
            return false;
        };
        map.get(&id).cloned()
    };
    let Some(control) = control else {
        return false;
    };
    control.cancel.store(true, Ordering::Relaxed);
    kill_pid(control.pid);
    true
}

pub fn spawn_fd_search(request: FindRequest, event_tx: Sender<Event>) {
    thread::spawn(move || match run_fd_search(&request, &event_tx) {
        Ok(FindRunOutcome::Done(entries)) => {
            let _ = event_tx.send(Event::Find(FindUpdate::Done {
                id: request.id,
                panel_id: request.panel_id,
                kind: request.kind,
                query: request.query,
                root: request.root,
                glob: request.glob,
                glob_pattern: request.glob_pattern,
                hidden: request.hidden,
                follow_symlinks: request.follow_symlinks,
                case_sensitive: request.case_sensitive,
                entries,
            }));
        }
        Ok(FindRunOutcome::Canceled) => {
            let _ = event_tx.send(Event::Find(FindUpdate::Canceled {
                id: request.id,
                panel_id: request.panel_id,
                kind: request.kind,
                query: request.query,
            }));
        }
        Err(err) => {
            let _ = event_tx.send(Event::Find(FindUpdate::Failed {
                id: request.id,
                panel_id: request.panel_id,
                kind: request.kind,
                query: request.query,
                error: err.to_string(),
            }));
        }
    });
}

pub fn spawn_rg_search(request: FindRequest, event_tx: Sender<Event>) {
    thread::spawn(move || match run_rg_search(&request, &event_tx) {
        Ok(FindRunOutcome::Done(entries)) => {
            let _ = event_tx.send(Event::Find(FindUpdate::Done {
                id: request.id,
                panel_id: request.panel_id,
                kind: request.kind,
                query: request.query,
                root: request.root,
                glob: request.glob,
                glob_pattern: request.glob_pattern,
                hidden: request.hidden,
                follow_symlinks: request.follow_symlinks,
                case_sensitive: request.case_sensitive,
                entries,
            }));
        }
        Ok(FindRunOutcome::Canceled) => {
            let _ = event_tx.send(Event::Find(FindUpdate::Canceled {
                id: request.id,
                panel_id: request.panel_id,
                kind: request.kind,
                query: request.query,
            }));
        }
        Err(err) => {
            let _ = event_tx.send(Event::Find(FindUpdate::Failed {
                id: request.id,
                panel_id: request.panel_id,
                kind: request.kind,
                query: request.query,
                error: err.to_string(),
            }));
        }
    });
}

fn run_fd_search(request: &FindRequest, event_tx: &Sender<Event>) -> Result<FindRunOutcome> {
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
    let cancel = Arc::new(AtomicBool::new(false));
    register_find_control(request.id, child.id(), cancel.clone());
    let _guard = FindControlGuard { id: request.id };

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
        if cancel.load(Ordering::Relaxed) {
            kill_pid(child.id());
            let _ = child.wait();
            return Ok(FindRunOutcome::Canceled);
        }

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
                kind: request.kind,
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
    if cancel.load(Ordering::Relaxed) {
        return Ok(FindRunOutcome::Canceled);
    }
    if !status.success() {
        let stderr_text = stderr_text.trim();
        if stderr_text.is_empty() {
            bail!("fd exited with status {status}");
        }
        bail!("fd failed: {stderr_text}");
    }

    Ok(FindRunOutcome::Done(entries))
}

fn run_rg_search(request: &FindRequest, event_tx: &Sender<Event>) -> Result<FindRunOutcome> {
    let mut command = ProcessCommand::new("rg");
    command
        .arg("--vimgrep")
        .arg("--color")
        .arg("never")
        .arg("--no-messages");
    if request.hidden {
        command.arg("--hidden");
    }
    if request.case_sensitive {
        command.arg("--case-sensitive");
    } else {
        command.arg("--ignore-case");
    }
    if let Some(glob) = request.glob_pattern.as_deref() {
        if !glob.trim().is_empty() {
            command.arg("--glob").arg(glob);
        }
    }
    command
        .arg("--")
        .arg(request.query.as_str())
        .arg(request.root.as_os_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|err| anyhow::anyhow!("failed to start rg: {err}"))?;
    let cancel = Arc::new(AtomicBool::new(false));
    register_find_control(request.id, child.id(), cancel.clone());
    let _guard = FindControlGuard { id: request.id };

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to capture rg stdout"))?;
    let mut stderr = child.stderr.take();
    let mut reader = BufReader::new(stdout);
    let mut entries = Vec::new();
    let mut matches = 0usize;
    let mut last_progress = Instant::now()
        .checked_sub(FIND_PROGRESS_INTERVAL)
        .unwrap_or_else(Instant::now);

    loop {
        if cancel.load(Ordering::Relaxed) {
            kill_pid(child.id());
            let _ = child.wait();
            return Ok(FindRunOutcome::Canceled);
        }

        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            break;
        }

        if let Some((path, line_no, snippet)) = parse_rg_vimgrep_line(line.trim_end()) {
            let entry =
                content_entry_from_match(path.as_path(), line_no, snippet.as_str(), &request.root);
            entries.push(entry);
            matches = matches.saturating_add(1);
            if last_progress.elapsed() >= FIND_PROGRESS_INTERVAL {
                let _ = event_tx.send(Event::Find(FindUpdate::Progress {
                    id: request.id,
                    panel_id: request.panel_id,
                    kind: request.kind,
                    query: request.query.clone(),
                    matches,
                }));
                last_progress = Instant::now();
            }
        }
    }

    let status = child
        .wait()
        .map_err(|err| anyhow::anyhow!("failed waiting for rg: {err}"))?;
    let mut stderr_text = String::new();
    if let Some(stderr_pipe) = stderr.as_mut() {
        let _ = stderr_pipe.read_to_string(&mut stderr_text);
    }

    if cancel.load(Ordering::Relaxed) {
        return Ok(FindRunOutcome::Canceled);
    }
    if status.code() == Some(1) {
        return Ok(FindRunOutcome::Done(entries));
    }
    if !status.success() {
        let stderr_text = stderr_text.trim();
        if stderr_text.is_empty() {
            bail!("rg exited with status {status}");
        }
        bail!("rg failed: {stderr_text}");
    }

    Ok(FindRunOutcome::Done(entries))
}

fn parse_rg_vimgrep_line(line: &str) -> Option<(PathBuf, usize, String)> {
    if line.trim().is_empty() {
        return None;
    }
    let mut head_parts = line.rsplitn(4, ':');
    let snippet = head_parts.next()?.to_string();
    let _column = head_parts.next()?.parse::<usize>().ok()?;
    let line_no = head_parts.next()?.parse::<usize>().ok()?;
    let path_raw = head_parts.next()?;
    Some((PathBuf::from(path_raw), line_no, snippet))
}

fn content_entry_from_match(path: &Path, line_no: usize, snippet: &str, root: &Path) -> FsEntry {
    let metadata = fs::symlink_metadata(path).ok();
    let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let display_path = canonical_path
        .strip_prefix(root)
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|_| canonical_path.to_string_lossy().to_string());
    let normalized_snippet = clamp_snippet(snippet.trim(), CONTENT_SNIPPET_MAX_CHARS);

    let size_bytes = metadata
        .as_ref()
        .map(|meta| {
            if meta.file_type().is_file() {
                meta.len()
            } else {
                0
            }
        })
        .unwrap_or(0);
    let modified_at = metadata.as_ref().and_then(|meta| meta.modified().ok());
    let is_executable = metadata.as_ref().map(|meta| is_exec(meta)).unwrap_or(false);

    FsEntry {
        name: format!("{display_path}:{line_no}:{normalized_snippet}"),
        path: canonical_path.clone(),
        entry_type: FsEntryType::File,
        size_bytes,
        modified_at,
        is_executable,
        is_hidden: canonical_path
            .file_name()
            .map(|name| name.to_string_lossy().starts_with('.'))
            .unwrap_or(false),
        is_virtual: false,
    }
}

fn clamp_snippet(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let chars: Vec<char> = input.chars().collect();
    if chars.len() <= max_chars {
        return input.to_string();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }
    let mut out = String::with_capacity(max_chars);
    for ch in chars.into_iter().take(max_chars - 3) {
        out.push(ch);
    }
    out.push_str("...");
    out
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

fn find_controls() -> &'static Mutex<HashMap<u64, FindControl>> {
    static MAP: OnceLock<Mutex<HashMap<u64, FindControl>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_find_control(id: u64, pid: u32, cancel: Arc<AtomicBool>) {
    if let Ok(mut map) = find_controls().lock() {
        map.insert(id, FindControl { cancel, pid });
    }
}

fn unregister_find_control(id: u64) {
    if let Ok(mut map) = find_controls().lock() {
        map.remove(&id);
    }
}

#[cfg(unix)]
fn kill_pid(pid: u32) {
    // SAFETY: passing process id from std::process::Child::id(), no borrowed pointers.
    let _ = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
}

#[cfg(not(unix))]
fn kill_pid(_pid: u32) {}

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
    use super::{
        ParsedContentSearchInput, ParsedFindInput, clamp_snippet, parse_content_search_input,
        parse_find_input, parse_rg_vimgrep_line,
    };
    use std::path::PathBuf;

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

    #[test]
    fn parse_content_search_input_parses_flags() {
        let parsed = parse_content_search_input(
            "needle --glob *.rs --hidden --case-sensitive",
            false,
            false,
        )
        .unwrap();
        assert_eq!(
            parsed,
            ParsedContentSearchInput {
                pattern: "needle".to_string(),
                glob_pattern: Some("*.rs".to_string()),
                hidden: true,
                case_sensitive: true,
            }
        );
    }

    #[test]
    fn parse_content_search_input_supports_glob_equals_and_case_toggle() {
        let parsed =
            parse_content_search_input("x --glob=src/** --ignore-case --no-hidden", true, true)
                .unwrap();
        assert_eq!(parsed.pattern, "x");
        assert_eq!(parsed.glob_pattern, Some("src/**".to_string()));
        assert!(!parsed.hidden);
        assert!(!parsed.case_sensitive);
    }

    #[test]
    fn parse_content_search_input_rejects_unknown_flag() {
        let err = parse_content_search_input("foo --wat", false, false).unwrap_err();
        assert!(
            err.to_string().contains("unknown content-search option"),
            "unexpected err: {err}"
        );
    }

    #[test]
    fn parse_rg_vimgrep_line_extracts_fields() {
        let parsed = parse_rg_vimgrep_line("/tmp/a.rs:42:7:let needle = true;").unwrap();
        assert_eq!(parsed.0, PathBuf::from("/tmp/a.rs"));
        assert_eq!(parsed.1, 42);
        assert_eq!(parsed.2, "let needle = true;");
    }

    #[test]
    fn clamp_snippet_limits_output() {
        let out = clamp_snippet("abcdefghijklmnopqrstuvwxyz", 10);
        assert_eq!(out, "abcdefg...");
    }
}
