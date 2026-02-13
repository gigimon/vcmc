# VCMC

VCMC is a fast terminal file manager inspired by Midnight Commander, implemented in Rust with `ratatui` and `crossterm`.

## Current Status

Implemented:
- two-panel layout with active panel focus
- navigation (`Tab`, `Up/Down`, `Enter`, `Backspace`, `Home`, `~`)
- command line (`:`) with `cd` and shell command execution
- interactive shell mode (`Ctrl+O`) with safe TUI suspend/resume
- file operations (`copy`, `move`, `delete`, `mkdir`) via background jobs
- MC-like multi-select (`Space/Ins`, range selection, select/deselect by mask, invert)
- batch `F5/F6/F8` over selected items with preflight checks and summary confirm
- interactive conflict matrix for `copy/move` (`overwrite/skip/rename/newer` + `apply to all`)
- incremental search (`/`) per panel
- per-panel sorting (`F2`)
- stable table layout (`Name | Size | Modified`) with narrow-width fallback
- unified dialog kit with buttons, focus, and keyboard navigation
- fullscreen viewer (`F3`) with smart text/binary-like fallback
- viewer+ (`F3`) with `text/hex` mode toggle and in-view search (`/`, `n`, `N`)
- external editor integration (`F4`) with `$EDITOR` or saved chooser fallback
- context-sensitive footer menu (MC-style) for `Normal`, `Selection`, `Dialog`, and `Viewer` modes
- top menu bar (MC-like) with keyboard navigation and action groups
- backend abstraction with SFTP panel mode (top menu: `Left/Right -> Connect SFTP`)
- archive VFS panel mode for `zip/tar/tar.gz` (browse + copy out)
- `fd`-powered find with async progress and panelized results view

## Run

```bash
cargo run
```

Smoke/performance check (non-interactive):

```bash
cargo run -- --smoke
```

## Hotkeys

General:
- `Tab`: switch active panel
- `Up/Down`: move current row
- `Enter`: open selected directory
- `Backspace`: go to parent directory
- `Home` or `~`: go to home directory
- `/`: start incremental search for active panel
- `:`: open command line (`cd`, shell commands, path jump)
- `Ctrl+O`: open interactive local shell in current directory
- `F2`: cycle sort mode (`name -> size -> mtime`)
- `F3`: open viewer for current file
- `F4`: open external editor for current file (`$EDITOR` or saved editor)
- `F5`: copy (`selection -> batch`, otherwise current item)
- `F6`: move (`selection -> batch`, otherwise current item)
- `F7`: create directory
- `F8`: delete (`selection -> batch`, otherwise current item)
- `F9`: open top menu (`Left`, `Options`, `Right`)
- `F10` or `q`: quit
- `Alt+L/O/R`: open top menu directly on specific group
- find dialog: `F9 -> Left/Right -> Find (fd)` then `pattern [--glob] [--hidden] [--follow]`
- editor chooser: `F9 -> Options -> Editor Settings`
- on local panel: `Enter` on archive file (`.zip`, `.tar`, `.tar.gz`, `.tgz`) opens archive VFS
- in archive VFS: `Backspace` at `/` closes archive and returns to local panel path

Selection:
- `Space` / `Ins`: toggle mark on current row
- `Shift+Up/Down`: range select from anchor
- `+`: select by mask (`*`, `?`)
- `-`: deselect by mask
- `*`: invert selection

Dialog controls:
- `Tab` / `Shift+Tab`: move button focus
- `Left/Right`: move button focus
- `Enter`: activate focused button
- `Esc`: cancel/close dialog
- `Alt+<letter>`: button accelerator (`Alt+Y`, `Alt+N`, `Alt+A`, `Alt+C`, ...)
- conflict dialog: `Alt+O` overwrite, `Alt+S` skip, `Alt+R` rename, `Alt+N` newer, `Alt+W/K/A` for `*All`, `Alt+C` cancel

Viewer controls:
- `F2`: toggle mode (`text` / `hex`)
- `/`: search in current viewer mode
- `n` / `N`: next / previous search match
- `Up/Down`: scroll line-by-line
- `PgUp/PgDn`: scroll page-by-page
- `Home/End`: jump to top/bottom
- `Esc` or `F3` or `q`: close viewer

SFTP connect dialog (`Left/Right -> Connect SFTP`):
- `user@host:port/path auth=agent`
- `user@host:port/path auth=password password=...`
- `user@host:port/path auth=key key=/path/to/private_key [passphrase=...]`
- `local` to switch active panel back to local filesystem

Remote workflow:
- connect target panel via `F9 -> Left/Right -> Connect SFTP` and one of the formats above
- use `Tab` to switch between local/remote panels
- `F5/F6/F8` use the same job model for `local<->sftp` and `sftp->sftp`

Archive workflow:
- open archive from selected file via `Enter` or `F9 -> Left/Right -> Archive VFS`
- navigate inside archive with regular panel keys
- top virtual item `..` exits archive VFS back to local panel
- copy from archive to local/sftp with `F5` (including batch selection)

Find workflow (`fd`):
- start via `F9 -> Left/Right -> Find (fd)` on local panel
- enter query as `pattern [--glob] [--hidden] [--follow]`
- search runs async and shows live match counter in footer
- results are shown in panelized view (`[fd:...]` in panel title)
- `Enter` on result jumps to source path (`file -> parent dir`, `dir -> open dir`)
- top virtual item `..` exits find-results view back to normal directory listing

Editor workflow:
- if `$EDITOR` is set, `F4` uses it
- if `$EDITOR` is unset, VCMC auto-detects available editors and asks to choose on first use
- selected editor is saved in config (`$XDG_CONFIG_HOME/vcmc/config.toml` or `~/.config/vcmc/config.toml`)
- saved editor can be changed anytime via `F9 -> Options -> Editor Settings`

## Architecture

- UI thread: event loop (`input`, `tick`, `resize`, `worker updates`)
- worker pool: background execution for long-running filesystem operations
- filesystem adapter: path normalization, listing, sorting, conflict handling
- backend abstraction: `LocalFs` + `SftpFs` (`ssh2`)
- conflict strategy: interactive matrix with per-item or global policy (`overwrite/skip/rename/newer`)

Security notes:
- `auth=password` from the dialog keeps password in process memory; prefer `auth=agent` or key-based auth
- `auth=key` supports optional `passphrase=...` in connect input
- connection retries/timeouts are enabled for SFTP connect path

## Scope and Limitations

- POSIX-first (`macOS`, `Linux`) for v1
- delete is permanent (no Trash integration)
- command line shell execution is available only on local backend
- viewer preview reads up to `256 KB` per file in v1
- SFTP backend uses short connection retries with timeout guards
- SFTP smoke checks are optional and run only when `VCMC_SFTP_SMOKE_*` env is configured
- archive VFS is read-only in v1 (no create/delete/move/write inside archive)
- viewer reads are preview-limited (`256 KB`) on all backends (local/sftp/archive)
- find via `fd` requires external `fd` binary available in `PATH`

## Known UX Constraints

- binary-like detection uses heuristic (NUL/non-printable ratio) and may misclassify edge cases
- viewer text rendering truncates very long lines (`512` chars) for stable TUI layout
- dialog mode uses keyboard-first interaction only (no mouse support)
- there is no internal editor yet (external `$EDITOR` only)
- if `$EDITOR` is set, it has priority over saved editor setting
- editor currently operates on local files only (viewer works for local/sftp/archive preview)
- SFTP smoke integration requires explicit test host env (not auto-run by default)
- archive VFS open is currently local-only (no direct open from SFTP file yet)
- copy into archive VFS is not implemented yet (copy out only)
- find via `fd` is currently local-only (no remote `sftp` search runner in v1)

## Backlog (next)

- internal editor mode
- tabs/bookmarks/favorites
- archive write/update operations
- optional Trash mode for safer delete behavior
- configurable keymap
