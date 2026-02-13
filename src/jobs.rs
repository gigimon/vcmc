use std::thread;

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, unbounded};
use tracing::warn;

use crate::backend::{FsBackend, backend_from_spec};
use crate::model::{Event, JobRequest, JobStatus, JobUpdate, SortMode};

pub struct WorkerPool {
    request_tx: Sender<JobRequest>,
    handles: Vec<thread::JoinHandle<()>>,
}

impl WorkerPool {
    pub fn new(worker_count: usize, event_tx: Sender<Event>) -> Self {
        let (request_tx, request_rx) = unbounded::<JobRequest>();
        let mut handles = Vec::new();

        for _ in 0..worker_count.max(1) {
            let worker_rx = request_rx.clone();
            let worker_event_tx = event_tx.clone();
            let handle = thread::spawn(move || worker_loop(worker_rx, worker_event_tx));
            handles.push(handle);
        }

        Self {
            request_tx,
            handles,
        }
    }

    pub fn submit(&self, request: JobRequest) -> Result<()> {
        self.request_tx.send(request)?;
        Ok(())
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        let (placeholder_tx, _) = unbounded::<JobRequest>();
        let old_tx = std::mem::replace(&mut self.request_tx, placeholder_tx);
        drop(old_tx);

        for handle in self.handles.drain(..) {
            if handle.join().is_err() {
                warn!("worker thread terminated with panic");
            }
        }
    }
}

fn worker_loop(request_rx: Receiver<JobRequest>, event_tx: Sender<Event>) {
    for request in request_rx {
        if event_tx
            .send(Event::Job(JobUpdate {
                id: request.id,
                batch_id: request.batch_id,
                kind: request.kind,
                status: JobStatus::Running,
                source: request.source.clone(),
                destination: request.destination.clone(),
                current_item: Some(source_label(&request.source)),
                batch_completed: None,
                batch_total: None,
                message: Some("running".to_string()),
            }))
            .is_err()
        {
            break;
        }

        let mut send_progress = |current_item: String, completed: usize, total: usize| -> bool {
            event_tx
                .send(Event::Job(JobUpdate {
                    id: request.id,
                    batch_id: request.batch_id,
                    kind: request.kind,
                    status: JobStatus::Running,
                    source: request.source.clone(),
                    destination: request.destination.clone(),
                    current_item: Some(current_item),
                    batch_completed: Some(completed),
                    batch_total: Some(total.max(1)),
                    message: Some("running".to_string()),
                }))
                .is_ok()
        };

        let outcome = execute_job(&request, &mut send_progress);
        let (status, destination, message) = match outcome {
            Ok(final_destination) => (
                JobStatus::Done,
                final_destination.or(request.destination.clone()),
                Some(format_job_success(&request)),
            ),
            Err(err) => (
                JobStatus::Failed,
                request.destination.clone(),
                Some(format_job_error(&request, &err)),
            ),
        };

        if event_tx
            .send(Event::Job(JobUpdate {
                id: request.id,
                batch_id: request.batch_id,
                kind: request.kind,
                status,
                source: request.source.clone(),
                destination,
                current_item: Some(source_label(&request.source)),
                batch_completed: None,
                batch_total: None,
                message,
            }))
            .is_err()
        {
            break;
        }
    }
}

fn execute_job<F>(request: &JobRequest, on_progress: &mut F) -> Result<Option<std::path::PathBuf>>
where
    F: FnMut(String, usize, usize) -> bool,
{
    let source_backend = backend_from_spec(&request.source_backend);
    match request.kind {
        crate::model::JobKind::Copy => {
            let destination = request
                .destination
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("copy requires destination"))?;
            let destination_backend_spec = request
                .destination_backend
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("copy requires destination backend"))?;
            let destination_backend = backend_from_spec(destination_backend_spec);

            let resolved = if &request.source_backend == destination_backend_spec {
                source_backend.copy_path(&request.source, destination)?
            } else {
                let total_items =
                    count_transfer_items(source_backend.as_ref(), &request.source)?.max(1);
                let mut completed_items = 0usize;
                copy_between_backends(
                    source_backend.as_ref(),
                    destination_backend.as_ref(),
                    &request.source,
                    destination,
                    &mut |copied_path| {
                        completed_items = completed_items.saturating_add(1);
                        let _ = on_progress(
                            source_label(copied_path),
                            completed_items.min(total_items),
                            total_items,
                        );
                    },
                )?;
                destination.clone()
            };
            Ok(Some(resolved))
        }
        crate::model::JobKind::Move => {
            let destination = request
                .destination
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("move requires destination"))?;
            let destination_backend_spec = request
                .destination_backend
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("move requires destination backend"))?;
            let destination_backend = backend_from_spec(destination_backend_spec);

            let resolved = if &request.source_backend == destination_backend_spec {
                source_backend.move_path(&request.source, destination)?
            } else {
                let total_items =
                    count_transfer_items(source_backend.as_ref(), &request.source)?.max(1);
                let mut completed_items = 0usize;
                copy_between_backends(
                    source_backend.as_ref(),
                    destination_backend.as_ref(),
                    &request.source,
                    destination,
                    &mut |copied_path| {
                        completed_items = completed_items.saturating_add(1);
                        let _ = on_progress(
                            source_label(copied_path),
                            completed_items.min(total_items),
                            total_items,
                        );
                    },
                )?;
                source_backend.remove_path(&request.source)?;
                destination.clone()
            };
            Ok(Some(resolved))
        }
        crate::model::JobKind::Delete => {
            source_backend.remove_path(&request.source)?;
            Ok(None)
        }
        crate::model::JobKind::Mkdir => {
            source_backend.create_dir(&request.source)?;
            Ok(None)
        }
    }
}

fn copy_between_backends(
    source_backend: &dyn FsBackend,
    destination_backend: &dyn FsBackend,
    source: &std::path::Path,
    destination: &std::path::Path,
    on_item_copied: &mut dyn FnMut(&std::path::Path),
) -> Result<()> {
    let source_entry = source_backend.stat_entry(source)?;
    if source_entry.entry_type == crate::model::FsEntryType::Directory {
        destination_backend.create_dir(destination)?;
        on_item_copied(source);
        for child in source_backend.list_dir(source, SortMode::Name, true)? {
            if child.is_virtual {
                continue;
            }
            let target = destination.join(&child.name);
            copy_between_backends(
                source_backend,
                destination_backend,
                child.path.as_path(),
                target.as_path(),
                on_item_copied,
            )?;
        }
        return Ok(());
    }

    let content = source_backend.read_file(source)?;
    destination_backend.write_file(destination, content.as_slice())?;
    on_item_copied(source);
    Ok(())
}

fn count_transfer_items(backend: &dyn FsBackend, source: &std::path::Path) -> Result<usize> {
    let entry = backend.stat_entry(source)?;
    if entry.entry_type != crate::model::FsEntryType::Directory {
        return Ok(1);
    }

    let mut total = 1usize;
    for child in backend.list_dir(source, SortMode::Name, true)? {
        if child.is_virtual {
            continue;
        }
        total = total.saturating_add(count_transfer_items(backend, child.path.as_path())?);
    }
    Ok(total)
}

fn format_job_success(request: &JobRequest) -> String {
    match request.kind {
        crate::model::JobKind::Copy => format!("copy done: {}", request.source.display()),
        crate::model::JobKind::Move => format!("move done: {}", request.source.display()),
        crate::model::JobKind::Delete => format!("delete done: {}", request.source.display()),
        crate::model::JobKind::Mkdir => format!("mkdir done: {}", request.source.display()),
    }
}

fn format_job_error(request: &JobRequest, err: &anyhow::Error) -> String {
    let dst = request
        .destination
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "-".to_string());

    match request.kind {
        crate::model::JobKind::Copy => format!(
            "copy failed: src={} dst={} reason={err}",
            request.source.display(),
            dst
        ),
        crate::model::JobKind::Move => format!(
            "move failed: src={} dst={} reason={err}",
            request.source.display(),
            dst
        ),
        crate::model::JobKind::Delete => {
            format!(
                "delete failed: target={} reason={err}",
                request.source.display()
            )
        }
        crate::model::JobKind::Mkdir => {
            format!(
                "mkdir failed: target={} reason={err}",
                request.source.display()
            )
        }
    }
}

fn source_label(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}
