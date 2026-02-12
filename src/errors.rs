#![allow(dead_code)]

use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("permission denied during {operation}: {path}")]
    PermissionDenied {
        operation: &'static str,
        path: PathBuf,
    },
    #[error("path not found during {operation}: {path}")]
    NotFound {
        operation: &'static str,
        path: PathBuf,
    },
    #[error("io error during {operation} for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid path for {operation}: {path} ({reason})")]
    InvalidPath {
        operation: &'static str,
        path: PathBuf,
        reason: String,
    },
    #[error("conflict during {operation}: {path} ({reason})")]
    Conflict {
        operation: &'static str,
        path: PathBuf,
        reason: String,
    },
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn from_io(operation: &'static str, path: PathBuf, source: io::Error) -> Self {
        match source.kind() {
            io::ErrorKind::PermissionDenied => Self::PermissionDenied { operation, path },
            io::ErrorKind::NotFound => Self::NotFound { operation, path },
            _ => Self::Io {
                operation,
                path,
                source,
            },
        }
    }

    pub fn invalid_path(
        operation: &'static str,
        path: impl Into<PathBuf>,
        reason: impl Into<String>,
    ) -> Self {
        Self::InvalidPath {
            operation,
            path: path.into(),
            reason: reason.into(),
        }
    }

    pub fn conflict(
        operation: &'static str,
        path: impl Into<PathBuf>,
        reason: impl Into<String>,
    ) -> Self {
        Self::Conflict {
            operation,
            path: path.into(),
            reason: reason.into(),
        }
    }
}
