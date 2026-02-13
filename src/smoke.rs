use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use crossbeam_channel::unbounded;

use crate::fs::FsAdapter;
use crate::jobs::WorkerPool;
use crate::model::{Event, JobKind, JobRequest, JobStatus, SortMode};

pub struct SmokeReport {
    pub temp_root: PathBuf,
    pub first_listing_ms: f64,
    pub navigation_avg_ms: f64,
    pub copy_submit_ms: f64,
    pub copy_total_ms: f64,
    pub ui_loop_iterations: u64,
    pub copied_files: usize,
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
                "copied_files: {}\n"
            ),
            self.temp_root.display(),
            self.first_listing_ms,
            self.navigation_avg_ms,
            self.copy_submit_ms,
            self.copy_total_ms,
            self.ui_loop_iterations,
            self.copied_files
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

    Ok(SmokeReport {
        temp_root: temp_root.to_path_buf(),
        first_listing_ms,
        navigation_avg_ms,
        copy_submit_ms,
        copy_total_ms,
        ui_loop_iterations,
        copied_files,
    })
}

struct SmokeWorkload {
    list_dir: PathBuf,
    navigation_paths: Vec<PathBuf>,
    copy_source: PathBuf,
    copy_destination_dir: PathBuf,
    copy_expected_files: usize,
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

    Ok(SmokeWorkload {
        list_dir,
        navigation_paths,
        copy_source: copy_source_dir,
        copy_destination_dir,
        copy_expected_files,
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
