#![allow(dead_code)]

use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("permission denied: {path}")]
    PermissionDenied { path: PathBuf },
    #[error("path not found: {path}")]
    NotFound { path: PathBuf },
    #[error("io error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub type AppResult<T> = Result<T, AppError>;
