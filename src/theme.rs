use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

use crate::model::{FsEntry, FsEntryType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ThemeStyle {
    pub fg: Option<ThemeColor>,
    pub bold: bool,
}

#[derive(Debug, Clone)]
pub struct DirColorsTheme {
    pub dir: ThemeStyle,
    pub link: ThemeStyle,
    pub exec: Option<ThemeStyle>,
    pub reset: ThemeStyle,
    pub exts: HashMap<String, ThemeStyle>,
}

impl DirColorsTheme {
    pub fn fallback() -> Self {
        Self {
            dir: ThemeStyle {
                fg: Some(ThemeColor::Blue),
                bold: false,
            },
            link: ThemeStyle {
                fg: Some(ThemeColor::Magenta),
                bold: false,
            },
            exec: None,
            reset: ThemeStyle::default(),
            exts: HashMap::new(),
        }
    }

    pub fn style_for_entry(&self, entry: &FsEntry) -> ThemeStyle {
        if entry.is_virtual {
            return ThemeStyle {
                fg: Some(ThemeColor::Yellow),
                bold: true,
            };
        }

        if let Some(ext) = extension_key(entry) {
            if let Some(style) = self.exts.get(ext.as_str()) {
                return *style;
            }
        }

        match entry.entry_type {
            FsEntryType::Directory => self.dir,
            FsEntryType::Symlink => self.link,
            FsEntryType::File if entry.is_executable => self.exec.unwrap_or_default(),
            _ => ThemeStyle::default(),
        }
    }

    fn apply_dircolors_text(&mut self, content: &str) {
        for raw_line in content.lines() {
            let line = raw_line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }

            let mut parts = line.split_whitespace();
            let Some(key) = parts.next() else {
                continue;
            };
            let Some(value) = parts.next() else {
                continue;
            };
            let Some(style) = parse_style_codes(value) else {
                continue;
            };
            self.apply_token(key, style);
        }
    }

    fn apply_ls_colors(&mut self, value: &str) {
        for item in value.split(':') {
            let Some((key, code)) = item.split_once('=') else {
                continue;
            };
            let Some(style) = parse_style_codes(code) else {
                continue;
            };
            self.apply_token(key, style);
        }
    }

    fn apply_token(&mut self, raw_key: &str, style: ThemeStyle) {
        let key = raw_key.trim();
        let upper = key.to_ascii_uppercase();
        match upper.as_str() {
            "DIR" | "DI" => self.dir = style,
            "LINK" | "LN" => self.link = style,
            "EXEC" | "EX" => self.exec = Some(style),
            "RESET" | "RS" => self.reset = style,
            _ => {
                if let Some(ext) = normalize_extension_key(key) {
                    self.exts.insert(ext, style);
                }
            }
        }
    }
}

pub fn load_theme_from_environment() -> DirColorsTheme {
    let mut theme = DirColorsTheme::fallback();

    if let Some(path) = discover_dircolors_path() {
        if let Ok(content) = fs::read_to_string(path) {
            theme.apply_dircolors_text(&content);
        }
    }

    if let Ok(ls_colors) = env::var("LS_COLORS") {
        if !ls_colors.trim().is_empty() {
            theme.apply_ls_colors(ls_colors.as_str());
        }
    }

    theme
}

fn discover_dircolors_path() -> Option<PathBuf> {
    if let Ok(path) = env::var("VCMC_DIRCOLORS_PATH") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    let mut candidates = Vec::new();
    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".dir_colors"));
        candidates.push(home.join(".dircolors"));
    }
    candidates.push(PathBuf::from("/etc/DIR_COLORS"));
    candidates.push(PathBuf::from("/etc/dircolors"));

    candidates.into_iter().find(|path| path.exists())
}

fn extension_key(entry: &FsEntry) -> Option<String> {
    let extension = entry.path.extension()?.to_string_lossy().to_lowercase();
    Some(format!("*.{extension}"))
}

fn normalize_extension_key(key: &str) -> Option<String> {
    let key = key.trim();
    if key.starts_with("*.") {
        if key.len() <= 2 {
            return None;
        }
        return Some(key.to_lowercase());
    }

    if key.starts_with('.') && key.len() > 1 {
        return Some(format!("*{}", key.to_lowercase()));
    }

    None
}

fn parse_style_codes(value: &str) -> Option<ThemeStyle> {
    let mut style = ThemeStyle::default();
    let mut seen = false;

    let codes: Vec<u16> = value
        .split(';')
        .filter_map(|part| part.trim().parse::<u16>().ok())
        .collect();

    let mut idx = 0usize;
    while idx < codes.len() {
        let code = codes[idx];
        match code {
            0 => {
                style = ThemeStyle::default();
                seen = true;
                idx += 1;
            }
            1 => {
                style.bold = true;
                seen = true;
                idx += 1;
            }
            22 => {
                style.bold = false;
                seen = true;
                idx += 1;
            }
            30..=37 | 90..=97 => {
                style.fg = map_sgr_color(code);
                seen = true;
                idx += 1;
            }
            38 => {
                if let Some((color, consumed)) = parse_extended_color(&codes, idx) {
                    style.fg = Some(color);
                    seen = true;
                    idx += consumed;
                } else {
                    idx += 1;
                }
            }
            39 => {
                style.fg = None;
                seen = true;
                idx += 1;
            }
            _ => {
                idx += 1;
            }
        }
    }
    if seen { Some(style) } else { None }
}

fn parse_extended_color(codes: &[u16], idx: usize) -> Option<(ThemeColor, usize)> {
    if idx + 1 >= codes.len() {
        return None;
    }

    match codes[idx + 1] {
        5 => {
            if idx + 2 >= codes.len() || codes[idx + 2] > 255 {
                return None;
            }
            Some((ThemeColor::Indexed(codes[idx + 2] as u8), 3))
        }
        2 => {
            if idx + 4 >= codes.len()
                || codes[idx + 2] > 255
                || codes[idx + 3] > 255
                || codes[idx + 4] > 255
            {
                return None;
            }
            Some((
                ThemeColor::Rgb(
                    codes[idx + 2] as u8,
                    codes[idx + 3] as u8,
                    codes[idx + 4] as u8,
                ),
                5,
            ))
        }
        _ => None,
    }
}

fn map_sgr_color(code: u16) -> Option<ThemeColor> {
    match code {
        30 => Some(ThemeColor::Black),
        31 => Some(ThemeColor::Red),
        32 => Some(ThemeColor::Green),
        33 => Some(ThemeColor::Yellow),
        34 => Some(ThemeColor::Blue),
        35 => Some(ThemeColor::Magenta),
        36 => Some(ThemeColor::Cyan),
        37 => Some(ThemeColor::White),
        90 => Some(ThemeColor::BrightBlack),
        91 => Some(ThemeColor::BrightRed),
        92 => Some(ThemeColor::BrightGreen),
        93 => Some(ThemeColor::BrightYellow),
        94 => Some(ThemeColor::BrightBlue),
        95 => Some(ThemeColor::BrightMagenta),
        96 => Some(ThemeColor::BrightCyan),
        97 => Some(ThemeColor::BrightWhite),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{DirColorsTheme, ThemeColor, normalize_extension_key, parse_style_codes};

    #[test]
    fn parse_dircolors_tokens_and_extensions() {
        let mut theme = DirColorsTheme::fallback();
        theme.apply_dircolors_text(
            r#"
DIR 01;34
LINK 01;36
EXEC 01;32
*.rs 00;33
RESET 0
"#,
        );
        assert_eq!(theme.dir.fg, Some(ThemeColor::Blue));
        assert_eq!(theme.link.fg, Some(ThemeColor::Cyan));
        assert_eq!(theme.exec.and_then(|s| s.fg), Some(ThemeColor::Green));
        assert_eq!(
            theme.exts.get("*.rs").and_then(|s| s.fg),
            Some(ThemeColor::Yellow)
        );
    }

    #[test]
    fn parse_style_codes_handles_bold_and_color() {
        let style = parse_style_codes("01;31").expect("style parsed");
        assert!(style.bold);
        assert_eq!(style.fg, Some(ThemeColor::Red));
    }

    #[test]
    fn parse_style_codes_handles_truecolor() {
        let style = parse_style_codes("01;38;2;255;121;198").expect("style parsed");
        assert!(style.bold);
        assert_eq!(style.fg, Some(ThemeColor::Rgb(255, 121, 198)));
    }

    #[test]
    fn normalize_extension_key_supports_dot_syntax() {
        assert_eq!(normalize_extension_key(".jpg"), Some("*.jpg".to_string()));
        assert_eq!(normalize_extension_key("*.png"), Some("*.png".to_string()));
    }
}
