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
- incremental search (`/`) per panel
- per-panel sorting (`F2`)
- stable table layout (`Name | Size | Modified`) with narrow-width fallback
- unified dialog kit with buttons, focus, and keyboard navigation
- fullscreen viewer (`F3`) with smart text/binary-like fallback
- external editor integration (`F4`) via `$EDITOR` with terminal suspend/resume
- context-sensitive footer menu (MC-style) for `Normal`, `Selection`, `Dialog`, and `Viewer` modes
- backend abstraction with SFTP panel mode (`F9` connect dialog)

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
- `F4`: open external editor (`$EDITOR`) for current file
- `F5`: copy (`selection -> batch`, otherwise current item)
- `F6`: move (`selection -> batch`, otherwise current item)
- `F7`: create directory
- `F8`: delete (`selection -> batch`, otherwise current item)
- `F9`: connect active panel to SFTP (or switch it back to `local`)
- `F10` or `q`: quit

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

Viewer controls:
- `Up/Down`: scroll line-by-line
- `PgUp/PgDn`: scroll page-by-page
- `Home/End`: jump to top/bottom
- `Esc` or `F3` or `q`: close viewer

SFTP connect dialog (`F9`):
- `user@host:port/path auth=agent`
- `user@host:port/path auth=password password=...`
- `user@host:port/path auth=key key=/path/to/private_key [passphrase=...]`
- `local` to switch active panel back to local filesystem

Remote workflow:
- connect active panel with `F9` and one of the formats above
- use `Tab` to switch between local/remote panels
- `F5/F6/F8` use the same job model for `local<->sftp` and `sftp->sftp`

## Architecture

- UI thread: event loop (`input`, `tick`, `resize`, `worker updates`)
- worker pool: background execution for long-running filesystem operations
- filesystem adapter: path normalization, listing, sorting, conflict handling
- backend abstraction: `LocalFs` + `SftpFs` (`ssh2`)
- conflict strategy: `copy/move` abort when destination already exists

Security notes:
- `auth=password` from the dialog keeps password in process memory; prefer `auth=agent` or key-based auth
- `auth=key` supports optional `passphrase=...` in connect input
- connection retries/timeouts are enabled for SFTP connect path

## Scope and Limitations

- POSIX-first (`macOS`, `Linux`) for v1
- delete is permanent (no Trash integration)
- overwrite flow is not implemented; conflicts are aborted explicitly
- command line shell execution is available only on local backend
- viewer preview reads up to `256 KB` per file in v1
- `F4` requires `$EDITOR` to be set in environment
- SFTP backend uses short connection retries with timeout guards
- SFTP smoke checks are optional and run only when `VCMC_SFTP_SMOKE_*` env is configured

## Known UX Constraints

- binary-like detection uses heuristic (NUL/non-printable ratio) and may misclassify edge cases
- viewer text rendering truncates very long lines (`512` chars) for stable TUI layout
- dialog mode uses keyboard-first interaction only (no mouse support)
- there is no internal editor yet (external `$EDITOR` only)
- there is no interactive conflict resolution matrix (`overwrite/rename/skip`) yet
- viewer/editor currently operate on local files only (SFTP preview/edit not yet wired)
- SFTP smoke integration requires explicit test host env (not auto-run by default)

## Backlog (next)

- hex-mode in viewer
- search in viewer
- internal editor mode
- tabs/bookmarks/favorites
- overwrite/rename/skip conflict resolution dialog
- archive operations (zip/tar)
- optional Trash mode for safer delete behavior
- configurable keymap
