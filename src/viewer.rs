use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::errors::{AppError, AppResult};
use crate::model::ViewerState;

const VIEWER_PREVIEW_LIMIT_BYTES: usize = 256 * 1024;
const BINARY_PREVIEW_LIMIT_BYTES: usize = 8 * 1024;
const MAX_TEXT_LINE_CHARS: usize = 512;
const TAB_WIDTH: usize = 4;
const BINARY_NON_PRINTABLE_THRESHOLD: f32 = 0.30;

pub fn load_viewer_state(path: PathBuf, title: String, byte_size: u64) -> AppResult<ViewerState> {
    let (bytes, truncated) = read_preview_bytes(&path, VIEWER_PREVIEW_LIMIT_BYTES)?;
    let is_binary_like = detect_binary_like(&bytes);
    let lines = if is_binary_like {
        build_binary_preview_lines(&path, byte_size, &bytes, truncated)
    } else {
        build_text_preview_lines(&bytes, truncated)
    };

    Ok(ViewerState {
        path,
        title,
        lines,
        scroll_offset: 0,
        is_binary_like,
        byte_size,
    })
}

fn read_preview_bytes(path: &Path, limit: usize) -> AppResult<(Vec<u8>, bool)> {
    let file =
        File::open(path).map_err(|err| AppError::from_io("viewer_open", path.into(), err))?;
    let mut reader = file.take((limit as u64) + 1);
    let mut bytes = Vec::with_capacity(limit.min(16 * 1024));
    reader
        .read_to_end(&mut bytes)
        .map_err(|err| AppError::from_io("viewer_read", path.into(), err))?;

    let truncated = bytes.len() > limit;
    if truncated {
        bytes.truncate(limit);
    }

    Ok((bytes, truncated))
}

fn detect_binary_like(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    if bytes.contains(&0) {
        return true;
    }

    let non_printable = bytes
        .iter()
        .filter(|&&byte| !is_text_friendly_byte(byte))
        .count();
    (non_printable as f32) / (bytes.len() as f32) > BINARY_NON_PRINTABLE_THRESHOLD
}

fn is_text_friendly_byte(byte: u8) -> bool {
    matches!(byte, b'\n' | b'\r' | b'\t') || (0x20..=0x7e).contains(&byte) || byte >= 0x80
}

fn build_text_preview_lines(bytes: &[u8], truncated: bool) -> Vec<String> {
    if bytes.is_empty() {
        return Vec::new();
    }

    let text = String::from_utf8_lossy(bytes);
    let normalized = normalize_newlines(text.as_ref());
    let mut lines: Vec<String> = normalized.split('\n').map(normalize_text_line).collect();

    if truncated {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!(
            "[preview truncated to {} KB]",
            VIEWER_PREVIEW_LIMIT_BYTES / 1024
        ));
    }

    lines
}

fn normalize_newlines(input: &str) -> String {
    input.replace("\r\n", "\n").replace('\r', "\n")
}

fn normalize_text_line(line: &str) -> String {
    clamp_line(expand_tabs(line), MAX_TEXT_LINE_CHARS)
}

fn expand_tabs(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut column = 0usize;

    for ch in line.chars() {
        if ch == '\t' {
            let spaces = TAB_WIDTH - (column % TAB_WIDTH);
            for _ in 0..spaces {
                output.push(' ');
            }
            column += spaces;
        } else {
            output.push(ch);
            column += 1;
        }
    }

    output
}

fn clamp_line(line: String, max_chars: usize) -> String {
    let count = line.chars().count();
    if count <= max_chars {
        return line;
    }

    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }

    let mut clamped = String::with_capacity(max_chars);
    for ch in line.chars().take(max_chars - 3) {
        clamped.push(ch);
    }
    clamped.push_str("...");
    clamped
}

fn build_binary_preview_lines(
    path: &Path,
    byte_size: u64,
    bytes: &[u8],
    truncated: bool,
) -> Vec<String> {
    let preview_len = bytes.len().min(BINARY_PREVIEW_LIMIT_BYTES);
    let preview = &bytes[..preview_len];
    let mut lines = vec![
        "Binary-like content detected.".to_string(),
        format!("Path: {}", path.display()),
        "Type: binary-like".to_string(),
        format!("Size: {}", format_bytes(byte_size)),
        format!(
            "Loaded preview: {} byte(s){}",
            bytes.len(),
            if truncated { " (truncated)" } else { "" }
        ),
        String::new(),
        format!(
            "Lossy preview (first {} KB):",
            BINARY_PREVIEW_LIMIT_BYTES / 1024
        ),
    ];

    if preview.is_empty() {
        lines.push("<empty>".to_string());
        return lines;
    }

    for chunk in preview.chunks(64) {
        lines.push(render_binary_chunk(chunk));
    }

    if bytes.len() > preview_len {
        lines.push(String::new());
        lines.push(format!(
            "[binary preview clipped to {} KB]",
            BINARY_PREVIEW_LIMIT_BYTES / 1024
        ));
    }

    lines
}

fn render_binary_chunk(chunk: &[u8]) -> String {
    chunk
        .iter()
        .map(|byte| {
            if (0x20..=0x7e).contains(byte) {
                *byte as char
            } else {
                '.'
            }
        })
        .collect()
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut size = bytes as f64;
    let mut idx = 0usize;
    while size >= 1024.0 && idx < UNITS.len() - 1 {
        size /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{bytes}{}", UNITS[idx])
    } else {
        format!("{size:.1}{}", UNITS[idx])
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_text_preview_lines, clamp_line, detect_binary_like, expand_tabs, load_viewer_state,
    };
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn detect_binary_by_nul_byte() {
        assert!(detect_binary_like(b"abc\0xyz"));
    }

    #[test]
    fn detect_binary_by_non_printable_ratio() {
        let bytes = [1u8, 2, 3, 4, b'a', b'b', b'c', b'd', b'e', b'f'];
        assert!(detect_binary_like(&bytes));
    }

    #[test]
    fn normalize_tabs_and_newlines_for_text_preview() {
        let lines = build_text_preview_lines(b"one\tcol\r\ntwo\rthree\n", false);
        assert_eq!(lines[0], "one col");
        assert_eq!(lines[1], "two");
        assert_eq!(lines[2], "three");
    }

    #[test]
    fn clamp_overlong_line() {
        let long = "a".repeat(1024);
        let clamped = clamp_line(long, 32);
        assert_eq!(clamped.chars().count(), 32);
        assert!(clamped.ends_with("..."));
    }

    #[test]
    fn expand_tabs_aligns_to_tab_stops() {
        assert_eq!(expand_tabs("a\tb"), "a   b");
        assert_eq!(expand_tabs("abcd\tx"), "abcd    x");
    }

    #[test]
    fn load_viewer_state_marks_preview_as_truncated() {
        let path = temp_file_path("truncated_preview.txt");
        let mut file = File::create(&path).expect("create temp file");
        let payload = "x".repeat(300 * 1024);
        file.write_all(payload.as_bytes()).expect("write payload");
        drop(file);

        let state = load_viewer_state(
            path.clone(),
            "truncated_preview.txt".to_string(),
            payload.len() as u64,
        )
        .expect("load viewer state");
        assert!(
            state
                .lines
                .iter()
                .any(|line| line.contains("[preview truncated to 256 KB]"))
        );

        let _ = fs::remove_file(path);
    }

    fn temp_file_path(name: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("valid time")
            .as_nanos();
        std::env::temp_dir().join(format!("vcmc_viewer_test_{timestamp}_{name}"))
    }
}
