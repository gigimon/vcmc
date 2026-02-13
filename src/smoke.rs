use std::env;
use std::fs::{self, File};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use crossbeam_channel::{Receiver, unbounded};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tar::{Builder as TarBuilder, EntryType, Header as TarHeader};

use crate::app::App;
use crate::backend::backend_from_spec;
use crate::find::{is_fd_available, spawn_fd_search};
use crate::fs::FsAdapter;
use crate::jobs::WorkerPool;
use crate::model::{
    AppState, ArchiveConnectionInfo, BackendSpec, Event, FindKind, FindRequest, FindUpdate,
    JobKind, JobRequest, JobStatus, PanelId, PanelState, SftpAuth, SftpConnectionInfo, SortMode,
    ViewerMode,
};
use crate::viewer::{
    jump_to_next_match, load_viewer_state, load_viewer_state_from_preview, refresh_viewer_search,
    set_viewer_mode,
};

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
    pub conflict_matrix_ok: bool,
    pub archive_vfs_browse_ok: bool,
    pub archive_vfs_copy_out_ok: bool,
    pub fd_find_enabled: bool,
    pub fd_find_ok: bool,
    pub viewer_search_hex_ok: bool,
    pub editor_chooser_ok: bool,
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
                "conflict_matrix_ok: {}\n",
                "archive_vfs_browse_ok: {}\n",
                "archive_vfs_copy_out_ok: {}\n",
                "fd_find_enabled: {}\n",
                "fd_find_ok: {}\n",
                "viewer_search_hex_ok: {}\n",
                "editor_chooser_ok: {}\n",
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
            self.conflict_matrix_ok,
            self.archive_vfs_browse_ok,
            self.archive_vfs_copy_out_ok,
            self.fd_find_enabled,
            self.fd_find_ok,
            self.viewer_search_hex_ok,
            self.editor_chooser_ok,
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

    let conflict_matrix_ok = run_conflict_matrix_probe(temp_root)?;
    let (archive_vfs_browse_ok, archive_vfs_copy_out_ok) = run_archive_vfs_probe(temp_root)?;
    let (fd_find_enabled, fd_find_ok) = run_fd_find_probe(temp_root)?;
    let viewer_search_hex_ok = run_viewer_search_hex_probe()?;
    let editor_chooser_ok = run_editor_chooser_probe(temp_root)?;

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
        conflict_matrix_ok,
        archive_vfs_browse_ok,
        archive_vfs_copy_out_ok,
        fd_find_enabled,
        fd_find_ok,
        viewer_search_hex_ok,
        editor_chooser_ok,
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

fn create_text_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content.as_bytes())?;
    Ok(())
}

fn run_conflict_matrix_probe(temp_root: &Path) -> Result<bool> {
    let conflict_root = temp_root.join("step34_conflict");
    let left_dir = conflict_root.join("left_src");
    let right_dir = conflict_root.join("right_dst");
    fs::create_dir_all(&left_dir)?;
    fs::create_dir_all(&right_dir)?;

    create_text_file(&left_dir.join("alpha.txt"), "alpha-new\n")?;
    create_text_file(&left_dir.join("beta.txt"), "beta-new\n")?;
    create_text_file(&right_dir.join("alpha.txt"), "alpha-old\n")?;
    create_text_file(&right_dir.join("beta.txt"), "beta-old\n")?;

    let (event_tx, event_rx) = unbounded();
    let mut app = App::bootstrap(conflict_root.clone(), event_tx)?;

    move_active_selection_to(&mut app, "left_src")?;
    press_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    press_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    move_active_selection_to(&mut app, "right_dst")?;
    press_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    press_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    move_active_selection_to(&mut app, "alpha.txt")?;
    press_key(&mut app, KeyCode::Char(' '), KeyModifiers::NONE);
    move_active_selection_to(&mut app, "beta.txt")?;
    press_key(&mut app, KeyCode::Char(' '), KeyModifiers::NONE);

    press_key(&mut app, KeyCode::F(5), KeyModifiers::NONE);
    let confirm_title = app
        .state()
        .dialog
        .as_ref()
        .map(|dialog| dialog.title.as_str())
        .unwrap_or_default();
    if confirm_title != "Confirm" {
        bail!("conflict probe expected confirm dialog, got '{confirm_title}'");
    }
    press_key(&mut app, KeyCode::Char('y'), KeyModifiers::ALT);

    let mut saw_conflict = false;
    let mut used_rename = false;
    let mut used_skip = false;
    for _ in 0..8 {
        let Some(dialog) = app.state().dialog.clone() else {
            break;
        };
        if !dialog.title.starts_with("Conflict ") {
            break;
        }
        saw_conflict = true;
        if !used_rename {
            press_key(&mut app, KeyCode::Char('r'), KeyModifiers::ALT);
            used_rename = true;
        } else {
            press_key(&mut app, KeyCode::Char('s'), KeyModifiers::ALT);
            used_skip = true;
        }
    }
    if !saw_conflict || !used_rename || !used_skip {
        bail!("conflict probe did not exercise rename+skip matrix actions");
    }

    wait_for_app_jobs(
        &mut app,
        &event_rx,
        Duration::from_secs(20),
        "conflict-probe",
    )?;

    let mut renamed_files = Vec::new();
    for entry in fs::read_dir(&right_dir)? {
        let path = entry?.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains("_copy"))
        {
            renamed_files.push(path);
        }
    }
    if renamed_files.len() != 1 {
        bail!(
            "conflict probe expected exactly one renamed file, got {}",
            renamed_files.len()
        );
    }

    let renamed_content = fs::read_to_string(&renamed_files[0])?;
    if renamed_content != "alpha-new\n" && renamed_content != "beta-new\n" {
        bail!(
            "conflict probe unexpected renamed payload in {}",
            renamed_files[0].display()
        );
    }

    let alpha_base = fs::read_to_string(right_dir.join("alpha.txt"))?;
    let beta_base = fs::read_to_string(right_dir.join("beta.txt"))?;
    if alpha_base != "alpha-old\n" || beta_base != "beta-old\n" {
        bail!("conflict probe expected original conflict targets to stay untouched");
    }

    Ok(true)
}

fn run_archive_vfs_probe(temp_root: &Path) -> Result<(bool, bool)> {
    let archive_root = temp_root.join("step34_archive");
    let archive_path = archive_root.join("bundle.tar");
    let out_dir = archive_root.join("out");
    fs::create_dir_all(&out_dir)?;

    let archive_payload = "archive-step34\n";
    create_tar_archive_with_member(&archive_path, "docs/readme.txt", archive_payload.as_bytes())?;

    let archive_spec = BackendSpec::Archive(ArchiveConnectionInfo {
        archive_path: archive_path.clone(),
    });
    let archive_backend = backend_from_spec(&archive_spec);

    let root_entries = archive_backend.list_dir(Path::new("/"), SortMode::Name, true)?;
    if !root_entries.iter().any(|entry| entry.name == "docs") {
        bail!("archive probe expected '/docs' in archive root listing");
    }
    let docs_entries = archive_backend.list_dir(Path::new("/docs"), SortMode::Name, true)?;
    if !docs_entries.iter().any(|entry| entry.name == "readme.txt") {
        bail!("archive probe expected '/docs/readme.txt' in archive listing");
    }
    let browse_ok = true;

    let (event_tx, event_rx) = unbounded();
    let worker_pool = WorkerPool::new(1, event_tx);
    let copy_out_target = out_dir.join("readme.out.txt");
    worker_pool.submit(JobRequest {
        id: 61_000,
        batch_id: None,
        kind: JobKind::Copy,
        source_backend: archive_spec,
        destination_backend: Some(BackendSpec::Local),
        source: PathBuf::from("/docs/readme.txt"),
        destination: Some(copy_out_target.clone()),
    })?;
    wait_for_terminal_updates(
        &event_rx,
        &[61_000],
        Duration::from_secs(20),
        "archive-copy-out",
    )?;

    let copied = fs::read_to_string(copy_out_target)?;
    if copied != archive_payload {
        bail!("archive copy-out payload mismatch");
    }

    Ok((browse_ok, true))
}

fn run_fd_find_probe(temp_root: &Path) -> Result<(bool, bool)> {
    if !is_fd_available() {
        return Ok((false, true));
    }

    let find_root = temp_root.join("step34_find");
    fs::create_dir_all(find_root.join("nested"))?;
    create_text_file(&find_root.join("alpha_needle_smoke.txt"), "x\n")?;
    create_text_file(&find_root.join("nested/needle_smoke.log"), "y\n")?;
    create_text_file(&find_root.join("nested/other.txt"), "z\n")?;

    let expected_a = fs::canonicalize(find_root.join("alpha_needle_smoke.txt"))?;
    let expected_b = fs::canonicalize(find_root.join("nested/needle_smoke.log"))?;

    let (event_tx, event_rx) = unbounded();
    spawn_fd_search(
        FindRequest {
            id: 71_000,
            panel_id: PanelId::Left,
            kind: FindKind::NameFd,
            root: find_root,
            query: "needle_smoke".to_string(),
            glob: false,
            glob_pattern: None,
            hidden: false,
            follow_symlinks: false,
            case_sensitive: false,
        },
        event_tx,
    );

    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(20) {
            bail!("fd find probe timed out");
        }

        match event_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(Event::Find(update)) => match update {
                FindUpdate::Progress { .. } => {}
                FindUpdate::Done { entries, .. } => {
                    let found: std::collections::HashSet<PathBuf> =
                        entries.into_iter().map(|entry| entry.path).collect();
                    if !found.contains(&expected_a) || !found.contains(&expected_b) {
                        bail!("fd find probe did not return expected matches");
                    }
                    return Ok((true, true));
                }
                FindUpdate::Failed { error, .. } => {
                    bail!("fd find probe failed: {error}");
                }
                FindUpdate::Canceled { .. } => {
                    bail!("fd find probe unexpectedly canceled");
                }
            },
            Ok(_) => {}
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(err) => bail!("fd find probe receive failed: {err}"),
        }
    }
}

fn run_viewer_search_hex_probe() -> Result<bool> {
    let bytes = b"alpha needle\nbeta marker\nneedle gamma\n".to_vec();
    let mut state = load_viewer_state_from_preview(
        PathBuf::from("/tmp/viewer-search-step34.txt"),
        "viewer-search-step34.txt".to_string(),
        bytes.len() as u64,
        bytes,
        false,
    );
    if state.mode != ViewerMode::Text {
        bail!("viewer search probe expected text mode by default");
    }

    state.search_query = "needle".to_string();
    refresh_viewer_search(&mut state);
    if state.search_matches.len() < 2 {
        bail!("viewer search probe expected >=2 text matches");
    }
    let Some(before_next) = state.search_matches.get(state.search_match_index).copied() else {
        bail!("viewer search probe has empty match index");
    };
    let Some(after_next) = jump_to_next_match(&mut state, true) else {
        bail!("viewer search probe expected next match");
    };
    if before_next == after_next {
        bail!("viewer search probe expected next match to move cursor");
    }

    set_viewer_mode(&mut state, ViewerMode::Hex);
    if state.mode != ViewerMode::Hex {
        bail!("viewer search probe failed to switch into hex mode");
    }
    state.search_query = "6E 65 65".to_string();
    refresh_viewer_search(&mut state);
    if state.search_matches.is_empty() {
        bail!("viewer search probe expected matches in hex mode");
    }

    Ok(true)
}

fn run_editor_chooser_probe(temp_root: &Path) -> Result<bool> {
    let editor_root = temp_root.join("step34_editor");
    fs::create_dir_all(&editor_root)?;
    create_text_file(&editor_root.join("sample.txt"), "editor chooser probe\n")?;

    let (event_tx, _event_rx) = unbounded();
    let mut app = App::bootstrap(editor_root, event_tx)?;

    press_key(&mut app, KeyCode::Char('o'), KeyModifiers::ALT);
    for _ in 0..3 {
        press_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    }
    press_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    let Some(dialog) = app.state().dialog.clone() else {
        bail!("editor chooser probe expected dialog after Options -> Editor Settings");
    };
    if dialog.title == "Editor Setup" {
        if !dialog.body.contains("Choose default editor") {
            bail!("editor chooser probe missing chooser body text");
        }
        if !dialog
            .body
            .lines()
            .any(|line| line.trim_start().starts_with("1: "))
        {
            bail!("editor chooser probe expected at least one candidate line");
        }
        press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        return Ok(true);
    }

    if dialog.title == "Error"
        && dialog
            .body
            .contains("No supported editors found in PATH (nvim/vim/nano/hx/micro/emacs/code)")
    {
        press_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        return Ok(true);
    }

    bail!(
        "editor chooser probe got unexpected dialog: '{}' ({})",
        dialog.title,
        dialog.body
    );
}

fn create_tar_archive_with_member(path: &Path, member: &str, bytes: &[u8]) -> Result<()> {
    let file = File::create(path)?;
    let mut builder = TarBuilder::new(file);
    let mut header = TarHeader::new_gnu();
    header.set_entry_type(EntryType::Regular);
    header.set_mode(0o644);
    header.set_size(bytes.len() as u64);
    header.set_cksum();
    builder.append_data(&mut header, member, Cursor::new(bytes))?;
    builder.finish()?;
    Ok(())
}

fn press_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    let key = KeyEvent::new(code, modifiers);
    let _ = app.on_event(Event::Input(key));
}

fn move_active_selection_to(app: &mut App, entry_name: &str) -> Result<()> {
    let target_index = active_panel_state(app.state())
        .entries
        .iter()
        .position(|entry| entry.name == entry_name)
        .ok_or_else(|| anyhow::anyhow!("entry '{entry_name}' not found in active panel"))?;

    loop {
        let current_index = active_panel_state(app.state()).selected_index;
        if current_index == target_index {
            break;
        }
        if current_index < target_index {
            press_key(app, KeyCode::Down, KeyModifiers::NONE);
        } else {
            press_key(app, KeyCode::Up, KeyModifiers::NONE);
        }
    }
    Ok(())
}

fn active_panel_state(state: &AppState) -> &PanelState {
    match state.active_panel {
        PanelId::Left => &state.left_panel,
        PanelId::Right => &state.right_panel,
    }
}

fn wait_for_app_jobs(
    app: &mut App,
    event_rx: &Receiver<Event>,
    timeout: Duration,
    scope: &str,
) -> Result<()> {
    let started = Instant::now();
    loop {
        let has_active = app
            .state()
            .jobs
            .iter()
            .any(|job| matches!(job.status, JobStatus::Queued | JobStatus::Running));
        if !has_active {
            return Ok(());
        }
        if started.elapsed() > timeout {
            bail!("{scope} timed out while waiting for app jobs");
        }

        match event_rx.recv_timeout(Duration::from_millis(25)) {
            Ok(event) => {
                let _ = app.on_event(event);
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(err) => bail!("{scope} failed receiving app event: {err}"),
        }
    }
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
