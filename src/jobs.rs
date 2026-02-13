use std::thread;

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, unbounded};
use tracing::warn;

use crate::fs::FsAdapter;
use crate::model::{Event, JobRequest, JobStatus, JobUpdate};

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
    let fs = FsAdapter::default();

    for request in request_rx {
        if event_tx
            .send(Event::Job(JobUpdate {
                id: request.id,
                batch_id: request.batch_id,
                kind: request.kind,
                status: JobStatus::Running,
                source: request.source.clone(),
                destination: request.destination.clone(),
                message: Some("running".to_string()),
            }))
            .is_err()
        {
            break;
        }

        let outcome = execute_job(&fs, &request);
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
                message,
            }))
            .is_err()
        {
            break;
        }
    }
}

fn execute_job(fs: &FsAdapter, request: &JobRequest) -> Result<Option<std::path::PathBuf>> {
    match request.kind {
        crate::model::JobKind::Copy => {
            let destination = request
                .destination
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("copy requires destination"))?;
            let resolved = fs.copy_path(&request.source, destination)?;
            Ok(Some(resolved))
        }
        crate::model::JobKind::Move => {
            let destination = request
                .destination
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("move requires destination"))?;
            let resolved = fs.move_path(&request.source, destination)?;
            Ok(Some(resolved))
        }
        crate::model::JobKind::Delete => {
            fs.remove_path(&request.source)?;
            Ok(None)
        }
        crate::model::JobKind::Mkdir => {
            fs.create_dir(&request.source)?;
            Ok(None)
        }
    }
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
