use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::errors::{AppError, AppResult};
use crate::model::{ViewerMode, ViewerState};

pub const VIEWER_PREVIEW_LIMIT_BYTES: usize = 256 * 1024;
const BINARY_PREVIEW_LIMIT_BYTES: usize = 8 * 1024;
const MAX_TEXT_LINE_CHARS: usize = 512;
const TAB_WIDTH: usize = 4;
const BINARY_NON_PRINTABLE_THRESHOLD: f32 = 0.30;
const HEX_LINE_BYTES: usize = 16;

pub fn load_viewer_state(path: PathBuf, title: String, byte_size: u64) -> AppResult<ViewerState> {
    let (bytes, truncated) = read_preview_bytes(&path, VIEWER_PREVIEW_LIMIT_BYTES)?;
    Ok(load_viewer_state_from_preview(
        path, title, byte_size, bytes, truncated,
    ))
}

pub fn load_viewer_state_from_preview(
    path: PathBuf,
    title: String,
    byte_size: u64,
    bytes: Vec<u8>,
    truncated: bool,
) -> ViewerState {
    let is_binary_like = detect_binary_like(&bytes);
    let text_lines = if is_binary_like {
        build_binary_preview_lines(&path, byte_size, &bytes, truncated)
    } else {
        build_text_preview_lines(&bytes, truncated)
    };
    let hex_lines = build_hex_preview_lines(&bytes, truncated);
    let mode = if is_binary_like {
        ViewerMode::Hex
    } else {
        ViewerMode::Text
    };
    let lines = lines_for_mode(mode, &text_lines, &hex_lines).to_vec();

    ViewerState {
        path,
        title,
        lines,
        text_lines,
        hex_lines,
        scroll_offset: 0,
        is_binary_like,
        byte_size,
        mode,
        preview_truncated: truncated,
        search_query: String::new(),
        search_matches: Vec::new(),
        search_match_index: 0,
    }
}

pub fn set_viewer_mode(state: &mut ViewerState, mode: ViewerMode) {
    if state.mode == mode {
        return;
    }
    state.mode = mode;
    state.lines = lines_for_mode(mode, &state.text_lines, &state.hex_lines).to_vec();
    state.scroll_offset = state.scroll_offset.min(state.lines.len().saturating_sub(1));
    refresh_viewer_search(state);
}

pub fn refresh_viewer_search(state: &mut ViewerState) {
    let query = state.search_query.trim().to_ascii_lowercase();
    if query.is_empty() {
        state.search_matches.clear();
        state.search_match_index = 0;
        return;
    }

    state.search_matches = state
        .lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            if line.to_ascii_lowercase().contains(query.as_str()) {
                Some(idx)
            } else {
                None
            }
        })
        .collect();
    if state.search_matches.is_empty() {
        state.search_match_index = 0;
        return;
    }

    state.search_match_index = state
        .search_matches
        .iter()
        .position(|&line_idx| line_idx >= state.scroll_offset)
        .unwrap_or(0);
    if let Some(line) = current_match_line(state) {
        state.scroll_offset = line;
    }
}

pub fn jump_to_next_match(state: &mut ViewerState, forward: bool) -> Option<usize> {
    if state.search_matches.is_empty() {
        return None;
    }
    let len = state.search_matches.len();
    state.search_match_index = if forward {
        (state.search_match_index + 1) % len
    } else if state.search_match_index == 0 {
        len - 1
    } else {
        state.search_match_index - 1
    };
    let line = state.search_matches[state.search_match_index];
    state.scroll_offset = line.min(state.lines.len().saturating_sub(1));
    Some(state.scroll_offset)
}

pub fn current_match_line(state: &ViewerState) -> Option<usize> {
    state.search_matches.get(state.search_match_index).copied()
}

pub fn detect_binary_like(bytes: &[u8]) -> bool {
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

fn build_hex_preview_lines(bytes: &[u8], truncated: bool) -> Vec<String> {
    if bytes.is_empty() {
        return vec!["<empty>".to_string()];
    }

    let mut lines = Vec::new();
    for (line_idx, chunk) in bytes.chunks(HEX_LINE_BYTES).enumerate() {
        let offset = line_idx * HEX_LINE_BYTES;
        let mut hex = String::new();
        for (idx, byte) in chunk.iter().enumerate() {
            if idx > 0 {
                hex.push(' ');
            }
            hex.push_str(format!("{byte:02X}").as_str());
        }
        if chunk.len() < HEX_LINE_BYTES {
            let pad = (HEX_LINE_BYTES - chunk.len()) * 3;
            hex.push_str(" ".repeat(pad).as_str());
        }

        let ascii: String = chunk
            .iter()
            .map(|byte| {
                if (0x20..=0x7e).contains(byte) {
                    *byte as char
                } else {
                    '.'
                }
            })
            .collect();
        lines.push(format!("{offset:08X}  {hex}  |{ascii}|"));
    }

    if truncated {
        lines.push(String::new());
        lines.push(format!(
            "[hex preview truncated to {} KB]",
            VIEWER_PREVIEW_LIMIT_BYTES / 1024
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

fn lines_for_mode<'a>(mode: ViewerMode, text: &'a [String], hex: &'a [String]) -> &'a [String] {
    match mode {
        ViewerMode::Text => text,
        ViewerMode::Hex => hex,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ViewerMode, build_text_preview_lines, clamp_line, detect_binary_like, expand_tabs,
        load_viewer_state, load_viewer_state_from_preview, set_viewer_mode,
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
        assert!(state.preview_truncated);
        assert!(
            state
                .lines
                .iter()
                .any(|line| line.contains("[preview truncated to 256 KB]"))
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn default_binary_mode_is_hex() {
        let state = load_viewer_state_from_preview(
            PathBuf::from("/tmp/test.bin"),
            "test.bin".to_string(),
            4,
            vec![0x00, 0x01, 0x02, 0x41],
            false,
        );
        assert_eq!(state.mode, ViewerMode::Hex);
        assert!(!state.lines.is_empty());
    }

    #[test]
    fn viewer_mode_switch_swaps_line_set() {
        let mut state = load_viewer_state_from_preview(
            PathBuf::from("/tmp/test.txt"),
            "test.txt".to_string(),
            11,
            b"hello\nworld".to_vec(),
            false,
        );
        assert_eq!(state.mode, ViewerMode::Text);
        let text_lines = state.lines.clone();
        set_viewer_mode(&mut state, ViewerMode::Hex);
        assert_eq!(state.mode, ViewerMode::Hex);
        assert_ne!(state.lines, text_lines);
    }

    fn temp_file_path(name: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("valid time")
            .as_nanos();
        std::env::temp_dir().join(format!("vcmc_viewer_test_{timestamp}_{name}"))
    }
}
