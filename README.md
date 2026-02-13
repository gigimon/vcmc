# VCMC

VCMC is a fast two-panel terminal file manager inspired by Midnight Commander.
Built in Rust with `ratatui` + `crossterm`, focused on responsive UI and predictable file operations.

## What Works

- Two-panel navigation with active panel focus (`Tab`)
- Local + SFTP backends
- Archive VFS (`zip`, `tar`, `tar.gz`, `tgz`) as read-only panel mode
- Async copy/move/delete/mkdir jobs (UI stays responsive)
- MC-like multi-selection (`Space/Ins`, range, mask select/deselect, invert)
- Interactive conflict matrix for copy/move (`overwrite/skip/rename/newer` + `*All`)
- Top menu bar (`Left`, `Options`, `Right`) with keyboard navigation
- Search files via external `fd` with panelized results
- Search text via external `rg` with panelized `file:line:snippet` results
- Fullscreen viewer with `text/hex` modes, in-view search, and match navigation
- External editor integration (`F4`) with:
  - `$EDITOR` priority
  - saved default editor fallback
  - first-run editor chooser when needed
- Command line (`:`) and shell mode (`Ctrl+O`)

## Requirements

- macOS or Linux
- Rust stable toolchain
- Optional but recommended:
  - `fd` for Find workflow
  - `rg` (`ripgrep`) for content-search workflow
  - available editors in `PATH` (`nvim`, `vim`, `nano`, `hx`, `micro`, `emacs`, `code`)

## Run

```bash
cargo run
```

Smoke check:

```bash
cargo run -- --smoke
```

## Keybindings

### General

- `Tab`: switch active panel
- `Up/Down`: move selection
- `Enter`: open selected directory / execute special panel item
- `Backspace`: go to parent directory
- `Home` or `~`: go to home directory
- `:`: open command line
- `Ctrl+O`: open interactive shell mode
- `/`: incremental search in active panel
- `F2`: cycle sort mode (`name -> size -> mtime`)
- `F3`: open viewer for selected file
- `F4`: open editor for selected local file
- `F5`: copy
- `F6`: move
- `F7`: mkdir
- `F8`: delete
- `F9`: open top menu
- `F10` or `q`: quit
- `Alt+L/O/R`: open top menu group directly (`Left` / `Options` / `Right`)

### Selection

- `Space` / `Ins`: toggle current item selection
- `Shift+Up/Down`: range select from anchor
- `+`: select by mask (`*`, `?`)
- `-`: deselect by mask
- `*`: invert selection

### Dialogs

- `Tab` / `Shift+Tab`: move focused button
- `Left/Right`: move focused button
- `Enter`: activate focused button
- `Esc`: cancel/close
- `Alt+<letter>`: button accelerator

### Viewer

- `F2`: toggle `text` / `hex`
- `/`: search in current viewer mode
- `n` / `N`: next / previous match
- `Up/Down`: line scroll
- `PgUp/PgDn`: page scroll
- `Home/End`: jump top/bottom
- `Esc` or `F3` or `q`: close viewer

## Top Menu

### Left / Right

Per-panel actions:

- Activate panel
- Home / Parent
- Copy / Move / Delete / Mkdir
- Connect SFTP (or disconnect if already connected)
- Command Line / Shell
- Search files
- Search text
- Archive VFS

### Options

- Sort
- Refresh
- Viewer Modes (help info)
- Editor Settings (choose and save default editor)

## Workflows

### SFTP

- Open `F9 -> Left/Right -> Connect SFTP`
- Enter address as `host[:port][/path]` or type `local` to switch back
- Then complete login/auth prompts (password or key path, depending on server auth methods)
- Use same copy/move/delete model between local and remote panels

### Archive VFS

- On local panel, select archive and press `Enter`, or use `F9 -> Left/Right -> Archive VFS`
- Archive opens as virtual panel mode (read-only)
- Top `..` exits archive mode back to local filesystem
- Supported v1 operations inside archive: browse + copy out (`archive -> local/sftp`)

### Search files (`fd`)

- Start from `F9 -> Left/Right -> Search files`
- Enter: `pattern [--glob] [--hidden] [--follow]`
- Results appear in panelized virtual view
- `Enter` on result:
  - directory: open it
  - file: jump to parent and select file
- Top `..` exits find results view
- If `fd` is missing, VCMC shows install hint

### Search text (`rg`, content)

- Start from `F9 -> Left/Right -> Search text`
- Enter: `pattern [--glob GLOB] [--hidden] [--case-sensitive|--ignore-case]`
- Search runs asynchronously with live match counter in footer
- Press `Esc` while running to cancel current search
- Results appear in panelized virtual view as `file:line:snippet`
- `Enter` on a result jumps to its file location in panel
- Top `..` exits search results view
- If `rg` is missing, VCMC shows install hint

### Viewer

- Works on local, SFTP, and archive backends
- Uses preview-limited reads (default `256 KB`)
- Binary-like files default to `hex` mode
- Search matches are highlighted; active match is emphasized

### Editor (`F4`)

- Local backend only
- Resolution order:
  1. `$EDITOR` (if set)
  2. saved editor from config
  3. chooser dialog from detected editors
- Saved setting can be changed via `F9 -> Options -> Editor Settings`

## Configuration

Editor config file:

- `$XDG_CONFIG_HOME/vcmc/config.toml`, or
- `~/.config/vcmc/config.toml`

Current key:

```toml
editor = "nvim"
```

Notes:

- If `$EDITOR` is set, it overrides saved config for current session.

## Command Line Mode (`:`)

Supported patterns:

- `cd`
- `cd <path>`
- direct path (`/tmp`, `./dir`, `../dir`, `~`)
- `sh` / `shell`
- non-interactive shell command (output opens in viewer)
- known interactive commands run in TTY mode (e.g. `vim`, `nvim`, `nano`, `less`, `top`, `ssh`)

## Limitations

- POSIX-first (macOS/Linux)
- Delete is permanent (no Trash)
- Archive VFS is read-only in v1
- Find via `fd` is local-only in v1
- Content search via `rg` is local-only in v1
- Viewer is preview-limited (`256 KB`) for all backends
- Editor works only on local backend
- No internal editor yet

## Architecture

- Main event loop for input/render/state transitions
- Worker pool for long operations (copy/move/delete/mkdir)
- Backend abstraction (`Local`, `Sftp`, `Archive`)
- Shared dialog framework for confirms/forms/conflicts
- Footer and top menu are mode-aware and keyboard-driven

## Iteration 6 Backlog (draft)

- Internal editor mode
- Directory compare/sync workflow
- Plugin hooks / extension points
