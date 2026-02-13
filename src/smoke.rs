use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use crossbeam_channel::{Receiver, unbounded};

use crate::backend::backend_from_spec;
use crate::fs::FsAdapter;
use crate::jobs::WorkerPool;
use crate::model::{
    BackendSpec, Event, JobKind, JobRequest, JobStatus, SftpAuth, SftpConnectionInfo, SortMode,
};
use crate::viewer::load_viewer_state;

pub struct SmokeReport {
    pub temp_root: PathBuf,
    pub first_listing_ms: f64,
    pub navigation_avg_ms: f64,
    pub copy_submit_ms: f64,
    pub copy_total_ms: f64,
    pub ui_loop_iterations: u64,
    pub copied_files: usize,
    pub batch_items: usize,
    pub batch_copy_total_ms: f64,
    pub batch_move_total_ms: f64,
    pub batch_delete_total_ms: f64,
    pub viewer_text_mode_ok: bool,
    pub viewer_binary_mode_ok: bool,
    pub viewer_scroll_probe_ok: bool,
    pub editor_roundtrip_ms: f64,
    pub sftp_smoke_enabled: bool,
    pub sftp_smoke_ok: bool,
    pub sftp_smoke_total_ms: f64,
}

impl SmokeReport {
    pub fn to_text(&self) -> String {
        format!(
            concat!(
                "SMOKE REPORT\n",
                "temp_root: {}\n",
                "first_listing_ms: {:.2}\n",
                "navigation_avg_ms: {:.2}\n",
                "copy_submit_ms: {:.3}\n",
                "copy_total_ms: {:.2}\n",
                "ui_loop_iterations_while_copy: {}\n",
                "copied_files: {}\n",
                "batch_items: {}\n",
                "batch_copy_total_ms: {:.2}\n",
                "batch_move_total_ms: {:.2}\n",
                "batch_delete_total_ms: {:.2}\n",
                "viewer_text_mode_ok: {}\n",
                "viewer_binary_mode_ok: {}\n",
                "viewer_scroll_probe_ok: {}\n",
                "editor_roundtrip_ms: {:.2}\n",
                "sftp_smoke_enabled: {}\n",
                "sftp_smoke_ok: {}\n",
                "sftp_smoke_total_ms: {:.2}\n"
            ),
            self.temp_root.display(),
            self.first_listing_ms,
            self.navigation_avg_ms,
            self.copy_submit_ms,
            self.copy_total_ms,
            self.ui_loop_iterations,
            self.copied_files,
            self.batch_items,
            self.batch_copy_total_ms,
            self.batch_move_total_ms,
            self.batch_delete_total_ms,
            self.viewer_text_mode_ok,
            self.viewer_binary_mode_ok,
            self.viewer_scroll_probe_ok,
            self.editor_roundtrip_ms,
            self.sftp_smoke_enabled,
            self.sftp_smoke_ok,
            self.sftp_smoke_total_ms
        )
    }
}

pub fn run_smoke() -> Result<SmokeReport> {
    let temp_root = make_temp_root();
    fs::create_dir_all(&temp_root)?;

    let result = run_smoke_inner(&temp_root);
    let cleanup = fs::remove_dir_all(&temp_root);

    if let Err(err) = cleanup {
        eprintln!(
            "warning: failed to cleanup smoke dir {}: {err}",
            temp_root.display()
        );
    }

    result
}

fn run_smoke_inner(temp_root: &Path) -> Result<SmokeReport> {
    let fs_adapter = FsAdapter::default();
    let workload = prepare_workload(temp_root)?;

    let first_listing_start = Instant::now();
    let _entries = fs_adapter.list_dir(&workload.list_dir, SortMode::Name, true)?;
    let first_listing_ms = first_listing_start.elapsed().as_secs_f64() * 1_000.0;

    let nav_rounds = 120usize;
    let navigation_start = Instant::now();
    for idx in 0..nav_rounds {
        let path = &workload.navigation_paths[idx % workload.navigation_paths.len()];
        let _ = fs_adapter.list_dir(path, SortMode::Name, true)?;
    }
    let navigation_avg_ms =
        (navigation_start.elapsed().as_secs_f64() * 1_000.0) / nav_rounds as f64;

    let (event_tx, event_rx) = unbounded();
    let worker_pool = WorkerPool::new(1, event_tx);

    let request = JobRequest {
        id: 1,
        batch_id: None,
        kind: JobKind::Copy,
        source_backend: BackendSpec::Local,
        destination_backend: Some(BackendSpec::Local),
        source: workload.copy_source.clone(),
        destination: Some(workload.copy_destination_dir.clone()),
    };

    let copy_start = Instant::now();
    let submit_start = Instant::now();
    worker_pool.submit(request)?;
    let copy_submit_ms = submit_start.elapsed().as_secs_f64() * 1_000.0;

    let mut ui_loop_iterations = 0_u64;
    let mut copy_done = false;
    while !copy_done {
        ui_loop_iterations = ui_loop_iterations.saturating_add(1);
        let _ = std::hint::black_box(ui_loop_iterations.wrapping_mul(17));

        match event_rx.recv_timeout(Duration::from_millis(2)) {
            Ok(Event::Job(update)) => match update.status {
                JobStatus::Done => copy_done = true,
                JobStatus::Failed => {
                    bail!(
                        "copy job failed: {}",
                        update
                            .message
                            .unwrap_or_else(|| "unknown failure".to_string())
                    );
                }
                JobStatus::Queued | JobStatus::Running => {}
            },
            Ok(_) => {}
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(err) => bail!("worker event channel failed: {err}"),
        }
    }
    let copy_total_ms = copy_start.elapsed().as_secs_f64() * 1_000.0;

    let copied_dir = workload
        .copy_destination_dir
        .join(workload.copy_source.file_name().unwrap_or_default());
    let copied_files = count_regular_files(&copied_dir)?;
    if copied_files != workload.copy_expected_files {
        bail!(
            "copied file count mismatch: expected {}, got {}",
            workload.copy_expected_files,
            copied_files
        );
    }

    let batch_item_count = workload.batch_sources.len();
    let mut next_job_id = 10_000_u64;

    let batch_copy_start = Instant::now();
    let mut batch_copy_job_ids = Vec::with_capacity(batch_item_count);
    for source in &workload.batch_sources {
        let file_name = source.file_name().ok_or_else(|| {
            anyhow::anyhow!("batch copy source has no file name: {}", source.display())
        })?;
        let destination = workload.batch_copy_destination.join(file_name);
        let id = next_job_id;
        next_job_id += 1;
        worker_pool.submit(JobRequest {
            id,
            batch_id: Some(21),
            kind: JobKind::Copy,
            source_backend: BackendSpec::Local,
            destination_backend: Some(BackendSpec::Local),
            source: source.clone(),
            destination: Some(destination),
        })?;
        batch_copy_job_ids.push(id);
    }
    wait_for_terminal_updates(
        &event_rx,
        &batch_copy_job_ids,
        Duration::from_secs(20),
        "batch-copy",
    )?;
    let batch_copy_total_ms = batch_copy_start.elapsed().as_secs_f64() * 1_000.0;

    let copied_batch_files = count_regular_files(&workload.batch_copy_destination)?;
    if copied_batch_files != batch_item_count {
        bail!(
            "batch copy mismatch: expected {} files in {}, got {}",
            batch_item_count,
            workload.batch_copy_destination.display(),
            copied_batch_files
        );
    }

    let batch_move_start = Instant::now();
    let mut batch_move_job_ids = Vec::with_capacity(batch_item_count);
    for source in &workload.batch_sources {
        let file_name = source.file_name().ok_or_else(|| {
            anyhow::anyhow!("batch move source has no file name: {}", source.display())
        })?;
        let source_path = workload.batch_copy_destination.join(file_name);
        let destination = workload.batch_move_destination.join(file_name);
        let id = next_job_id;
        next_job_id += 1;
        worker_pool.submit(JobRequest {
            id,
            batch_id: Some(22),
            kind: JobKind::Move,
            source_backend: BackendSpec::Local,
            destination_backend: Some(BackendSpec::Local),
            source: source_path,
            destination: Some(destination),
        })?;
        batch_move_job_ids.push(id);
    }
    wait_for_terminal_updates(
        &event_rx,
        &batch_move_job_ids,
        Duration::from_secs(20),
        "batch-move",
    )?;
    let batch_move_total_ms = batch_move_start.elapsed().as_secs_f64() * 1_000.0;

    let moved_batch_files = count_regular_files(&workload.batch_move_destination)?;
    if moved_batch_files != batch_item_count {
        bail!(
            "batch move mismatch: expected {} files in {}, got {}",
            batch_item_count,
            workload.batch_move_destination.display(),
            moved_batch_files
        );
    }
    let leftover_after_move = count_regular_files(&workload.batch_copy_destination)?;
    if leftover_after_move != 0 {
        bail!(
            "batch move left stale files in {}: {}",
            workload.batch_copy_destination.display(),
            leftover_after_move
        );
    }

    let batch_delete_start = Instant::now();
    let mut batch_delete_job_ids = Vec::with_capacity(batch_item_count);
    for target in &workload.batch_delete_targets {
        let id = next_job_id;
        next_job_id += 1;
        worker_pool.submit(JobRequest {
            id,
            batch_id: Some(23),
            kind: JobKind::Delete,
            source_backend: BackendSpec::Local,
            destination_backend: None,
            source: target.clone(),
            destination: None,
        })?;
        batch_delete_job_ids.push(id);
    }
    wait_for_terminal_updates(
        &event_rx,
        &batch_delete_job_ids,
        Duration::from_secs(20),
        "batch-delete",
    )?;
    let batch_delete_total_ms = batch_delete_start.elapsed().as_secs_f64() * 1_000.0;

    for target in &workload.batch_delete_targets {
        if target.try_exists()? {
            bail!("batch delete target still exists: {}", target.display());
        }
    }

    let text_size = fs::metadata(&workload.viewer_text_file)?.len();
    let text_state = load_viewer_state(
        workload.viewer_text_file.clone(),
        "viewer_text.txt".to_string(),
        text_size,
    )?;
    let viewer_text_mode_ok = !text_state.is_binary_like;
    if !viewer_text_mode_ok {
        bail!("viewer text probe incorrectly marked as binary-like");
    }

    let binary_size = fs::metadata(&workload.viewer_binary_file)?.len();
    let binary_state = load_viewer_state(
        workload.viewer_binary_file.clone(),
        "viewer_binary.bin".to_string(),
        binary_size,
    )?;
    let viewer_binary_mode_ok = binary_state.is_binary_like;
    if !viewer_binary_mode_ok {
        bail!("viewer binary probe was not detected as binary-like");
    }

    let viewer_scroll_probe_ok = probe_viewer_scroll(text_state.lines.len());
    if !viewer_scroll_probe_ok {
        bail!("viewer scroll probe failed for text sample");
    }

    let editor_roundtrip_start = Instant::now();
    run_editor_roundtrip_probe(&workload.viewer_text_file)?;
    let editor_roundtrip_ms = editor_roundtrip_start.elapsed().as_secs_f64() * 1_000.0;

    let (sftp_smoke_enabled, sftp_smoke_ok, sftp_smoke_total_ms) = run_optional_sftp_smoke()?;

    Ok(SmokeReport {
        temp_root: temp_root.to_path_buf(),
        first_listing_ms,
        navigation_avg_ms,
        copy_submit_ms,
        copy_total_ms,
        ui_loop_iterations,
        copied_files,
        batch_items: batch_item_count,
        batch_copy_total_ms,
        batch_move_total_ms,
        batch_delete_total_ms,
        viewer_text_mode_ok,
        viewer_binary_mode_ok,
        viewer_scroll_probe_ok,
        editor_roundtrip_ms,
        sftp_smoke_enabled,
        sftp_smoke_ok,
        sftp_smoke_total_ms,
    })
}

struct SmokeWorkload {
    list_dir: PathBuf,
    navigation_paths: Vec<PathBuf>,
    copy_source: PathBuf,
    copy_destination_dir: PathBuf,
    copy_expected_files: usize,
    batch_sources: Vec<PathBuf>,
    batch_copy_destination: PathBuf,
    batch_move_destination: PathBuf,
    batch_delete_targets: Vec<PathBuf>,
    viewer_text_file: PathBuf,
    viewer_binary_file: PathBuf,
}

fn prepare_workload(root: &Path) -> Result<SmokeWorkload> {
    let list_dir = root.join("list_big");
    fs::create_dir_all(&list_dir)?;
    create_many_files(&list_dir, 2_500)?;

    let nested_root = root.join("nav");
    fs::create_dir_all(&nested_root)?;
    let mut navigation_paths = Vec::new();
    navigation_paths.push(list_dir.clone());

    let mut current = nested_root;
    for depth in 0..10 {
        current = current.join(format!("d{depth}"));
        fs::create_dir_all(&current)?;
        create_single_file(&current.join("item.txt"), 256)?;
        navigation_paths.push(current.clone());
    }

    let copy_source_dir = root.join("copy_src_payload");
    let copy_destination_dir = root.join("copy_dst");
    fs::create_dir_all(&copy_source_dir)?;
    fs::create_dir_all(&copy_destination_dir)?;

    let copy_expected_files = 1_500usize;
    create_many_files(&copy_source_dir, copy_expected_files)?;

    let batch_source_dir = root.join("batch_src");
    let batch_copy_destination = root.join("batch_copy_dst");
    let batch_move_destination = root.join("batch_move_dst");
    let batch_delete_dir = root.join("batch_delete_src");
    fs::create_dir_all(&batch_source_dir)?;
    fs::create_dir_all(&batch_copy_destination)?;
    fs::create_dir_all(&batch_move_destination)?;
    fs::create_dir_all(&batch_delete_dir)?;

    let batch_items = 64usize;
    let mut batch_sources = Vec::with_capacity(batch_items);
    let mut batch_delete_targets = Vec::with_capacity(batch_items);
    for idx in 0..batch_items {
        let copy_file = batch_source_dir.join(format!("copy_{idx:03}.txt"));
        let delete_file = batch_delete_dir.join(format!("delete_{idx:03}.txt"));
        create_single_file(&copy_file, 96)?;
        create_single_file(&delete_file, 96)?;
        batch_sources.push(copy_file);
        batch_delete_targets.push(delete_file);
    }

    let viewer_text_file = root.join("viewer_text.txt");
    let viewer_binary_file = root.join("viewer_binary.bin");
    create_viewer_text_file(&viewer_text_file, 400)?;
    create_viewer_binary_file(&viewer_binary_file)?;

    Ok(SmokeWorkload {
        list_dir,
        navigation_paths,
        copy_source: copy_source_dir,
        copy_destination_dir,
        copy_expected_files,
        batch_sources,
        batch_copy_destination,
        batch_move_destination,
        batch_delete_targets,
        viewer_text_file,
        viewer_binary_file,
    })
}

fn create_many_files(dir: &Path, count: usize) -> Result<()> {
    for idx in 0..count {
        let path = dir.join(format!("item_{idx:04}.txt"));
        create_single_file(&path, 128)?;
    }
    Ok(())
}

fn create_single_file(path: &Path, size_bytes: usize) -> Result<u64> {
    let mut file = File::create(path)?;
    let chunk = vec![b'x'; 8 * 1024];
    let mut written = 0usize;
    while written < size_bytes {
        let remaining = size_bytes - written;
        let n = remaining.min(chunk.len());
        file.write_all(&chunk[..n])?;
        written += n;
    }
    Ok(written as u64)
}

fn create_viewer_text_file(path: &Path, lines: usize) -> Result<()> {
    let mut file = File::create(path)?;
    for idx in 0..lines {
        writeln!(file, "line-{idx:04}\tviewer smoke text")?;
    }
    Ok(())
}

fn create_viewer_binary_file(path: &Path) -> Result<()> {
    let mut file = File::create(path)?;
    let mut bytes = Vec::with_capacity(4096);
    for idx in 0..4096usize {
        bytes.push((idx % 256) as u8);
    }
    bytes[128] = 0;
    file.write_all(&bytes)?;
    Ok(())
}

fn make_temp_root() -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    env::temp_dir().join(format!("vcmc-smoke-{ts}"))
}

fn count_regular_files(path: &Path) -> Result<usize> {
    let mut count = 0usize;
    for entry in walkdir::WalkDir::new(path).follow_links(false).min_depth(1) {
        let entry = entry?;
        if entry.file_type().is_file() {
            count += 1;
        }
    }
    Ok(count)
}

fn probe_viewer_scroll(total_lines: usize) -> bool {
    if total_lines < 3 {
        return false;
    }
    let max_offset = total_lines.saturating_sub(1);
    let mut offset = 0usize;
    offset = (offset + 1).min(max_offset);
    offset = (offset + 1).min(max_offset);
    offset = offset.saturating_sub(1);
    offset == 1
}

fn run_editor_roundtrip_probe(path: &Path) -> Result<()> {
    let status = ProcessCommand::new("sh")
        .arg("-lc")
        .arg(format!("true {}", shell_escape_path(path)))
        .status()?;
    if !status.success() {
        bail!("editor roundtrip probe failed with status: {status}");
    }
    Ok(())
}

fn shell_escape_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("'{}'", raw.replace('\'', r#"'"'"'"#))
}

fn run_optional_sftp_smoke() -> Result<(bool, bool, f64)> {
    let Some(conn) = sftp_smoke_connection_from_env() else {
        return Ok((false, true, 0.0));
    };

    let started = Instant::now();
    let backend = backend_from_spec(&BackendSpec::Sftp(conn.clone()));
    let root = conn.root_path.clone();
    let _ = backend.list_dir(root.as_path(), SortMode::Name, true)?;

    let probe_dir = root.join(format!(
        ".vcmc-smoke-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis()
    ));
    backend.create_dir(probe_dir.as_path())?;

    let probe_file = probe_dir.join("probe.txt");
    let payload = b"vcmc-sftp-smoke";
    backend.write_file(probe_file.as_path(), payload)?;
    let roundtrip = backend.read_file(probe_file.as_path())?;
    if roundtrip.as_slice() != payload {
        bail!("sftp smoke roundtrip mismatch");
    }
    backend.remove_path(probe_file.as_path())?;
    backend.remove_path(probe_dir.as_path())?;

    Ok((true, true, started.elapsed().as_secs_f64() * 1_000.0))
}

fn sftp_smoke_connection_from_env() -> Option<SftpConnectionInfo> {
    let host = env::var("VCMC_SFTP_SMOKE_HOST").ok()?;
    let user = env::var("VCMC_SFTP_SMOKE_USER").ok()?;
    let root_path = env::var("VCMC_SFTP_SMOKE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    let port = env::var("VCMC_SFTP_SMOKE_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(22);

    let auth = match env::var("VCMC_SFTP_SMOKE_AUTH")
        .unwrap_or_else(|_| "agent".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "agent" => SftpAuth::Agent,
        "password" => SftpAuth::Password(env::var("VCMC_SFTP_SMOKE_PASSWORD").ok()?),
        "key" => SftpAuth::KeyFile {
            path: PathBuf::from(env::var("VCMC_SFTP_SMOKE_KEY").ok()?),
            passphrase: env::var("VCMC_SFTP_SMOKE_PASSPHRASE").ok(),
        },
        _ => return None,
    };

    Some(SftpConnectionInfo {
        host,
        user,
        port,
        root_path,
        auth,
    })
}

fn wait_for_terminal_updates(
    event_rx: &Receiver<Event>,
    expected_job_ids: &[u64],
    timeout: Duration,
    scope: &str,
) -> Result<()> {
    let expected: std::collections::HashSet<u64> = expected_job_ids.iter().copied().collect();
    let mut completed = std::collections::HashSet::new();
    let start = Instant::now();

    while completed.len() < expected.len() {
        if start.elapsed() > timeout {
            bail!(
                "{scope} timed out: completed {} out of {} jobs",
                completed.len(),
                expected.len()
            );
        }

        match event_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Event::Job(update)) => {
                if !expected.contains(&update.id) {
                    continue;
                }
                if !matches!(update.status, JobStatus::Done | JobStatus::Failed) {
                    continue;
                }
                if !completed.insert(update.id) {
                    continue;
                }
                if update.status == JobStatus::Failed {
                    bail!(
                        "{scope} failed for job {}: {}",
                        update.id,
                        update
                            .message
                            .unwrap_or_else(|| "unknown failure".to_string())
                    );
                }
            }
            Ok(_) => {}
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(err) => bail!("{scope} failed to receive job update: {err}"),
        }
    }

    Ok(())
}
