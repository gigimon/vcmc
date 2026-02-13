# VCMC

VCMC is a fast terminal file manager inspired by Midnight Commander, implemented in Rust with `ratatui` and `crossterm`.

## Current Status

Implemented:
- two-panel layout with active panel focus
- navigation (`Tab`, `Up/Down`, `Enter`, `Backspace`, `Home`, `~`)
- file operations (`copy`, `move`, `delete`, `mkdir`) via background jobs
- MC-like multi-select (`Space/Ins`, range selection, select/deselect by mask, invert)
- batch `F5/F6/F8` over selected items with preflight checks and summary confirm
- incremental search (`/`) per panel
- per-panel sorting (`F2`)
- stable table layout (`Name | Size | Modified`) with narrow-width fallback
- unified dialog kit with buttons, focus, and keyboard navigation
- context-sensitive footer menu (MC-style) for `Normal`, `Selection`, and `Dialog` modes

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
- `F2`: cycle sort mode (`name -> size -> mtime`)
- `F5`: copy (`selection -> batch`, otherwise current item)
- `F6`: move (`selection -> batch`, otherwise current item)
- `F7`: create directory
- `F8`: delete (`selection -> batch`, otherwise current item)
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

## Architecture

- UI thread: event loop (`input`, `tick`, `resize`, `worker updates`)
- worker pool: background execution for long-running filesystem operations
- filesystem adapter: path normalization, listing, sorting, conflict handling
- conflict strategy: `copy/move` abort when destination already exists

## Scope and Limitations

- POSIX-first (`macOS`, `Linux`) for v1
- delete is permanent (no Trash integration)
- overwrite flow is not implemented; conflicts are aborted explicitly
- no built-in preview panel yet

## Known UX Constraints

- footer actions are partially placeholders (`F1/F3/F4/F9` shown for MC parity, not implemented yet)
- dialog mode uses keyboard-first interaction only (no mouse support)
- there is no interactive conflict resolution matrix (`overwrite/rename/skip`) yet

## Backlog (next)

- preview panel and richer file metadata
- tabs/bookmarks/favorites
- overwrite/rename/skip conflict resolution dialog
- archive operations (zip/tar)
- optional Trash mode for safer delete behavior
- configurable keymap
