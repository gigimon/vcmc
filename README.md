# VCMC

VCMC is a fast terminal file manager inspired by Midnight Commander, implemented in Rust with `ratatui` and `crossterm`.

## Current Status

Implemented:
- two-panel layout with active panel focus
- navigation (`Tab`, `Up/Down`, `Enter`, `Backspace`, `Home`, `~`)
- file operations (`copy`, `move`, `delete`, `mkdir`) via background jobs
- delete confirmation modal and error alert modal
- incremental search (`/`) per panel
- per-panel sorting (`F2`)
- copy/move rename dialog (`F5`, `F6`)
- tabular list view (`Name | Size | Modified`) with colorized file types

## Run

```bash
cargo run
```

Smoke/performance check (non-interactive):

```bash
cargo run -- --smoke
```

## Hotkeys

- `Tab`: switch active panel
- `Up/Down`: move selection
- `Enter`: open selected directory
- `Backspace`: go to parent directory
- `Home` or `~`: go to home directory
- `/`: start incremental search for active panel
- `F2`: cycle sort mode (`name -> size -> mtime`)
- `F5`: copy with rename dialog
- `F6`: move with rename dialog
- `F7`: create directory
- `F8`: delete (with confirmation)
- `q`: quit

Modal controls:
- delete confirm: `y/Y` to confirm, `n/N`, `Esc`, or `Enter` to cancel
- rename dialog: `Enter` apply, `Esc` cancel
- alert modal: any key closes

## Architecture

- UI thread: event loop (`input`, `tick`, `resize`, `worker updates`)
- worker pool: background execution for long-running filesystem operations
- filesystem adapter: path normalization, listing, sorting, conflict handling
- conflict strategy: `copy/move` abort when destination already exists

## Scope and Limitations

- POSIX-first (`macOS`, `Linux`) for v1
- delete is permanent (no Trash integration)
- overwrite flow is not implemented; conflicts are aborted explicitly
- no archive/preview/multi-select/tabs yet

## Known Issues

- column widths are currently heuristic and not fully adaptive per terminal width
- no keyboard shortcut remapping yet
- no batch conflict resolution dialog yet

## Backlog (v2 candidates)

- overwrite/rename/skip conflict resolution dialog
- integrated file preview panel
- multi-select and bulk operations
- tabs/bookmarks/favorites
- archive operations (zip/tar)
- optional Trash mode for safer delete behavior
