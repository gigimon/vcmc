# Step 32: Viewer+ (улучшения просмотра)

## Цель
Расширить viewer до более практичного режима ежедневной работы: `text/hex`, поиск внутри viewer и preview-чтение для remote backend-ов.

## Решения
- В `src/model.rs` расширен `ViewerState`:
  - `mode: ViewerMode` (`text|hex`);
  - отдельные наборы строк `text_lines` и `hex_lines`;
  - состояние поиска (`search_query`, `search_matches`, `search_match_index`);
  - флаг `preview_truncated`.
- В `src/viewer.rs`:
  - добавлен hex renderer (offset + hex bytes + ascii);
  - добавлены функции переключения режима и управления поиском;
  - default mode для binary-like контента — `hex`.
- В `src/backend.rs`:
  - добавлен `FsBackend::read_file_preview(path, limit)` с лимитом и `truncated` флагом;
  - локальный, SFTP и archive backend-ы читают preview через ограниченный buffered read.
- В `src/app.rs`:
  - `F3` работает на всех backend-ах (local/sftp/archive), не только local;
  - добавлены команды viewer: `F2` mode toggle, `/` search prompt, `n/N` next/prev match.
- В `src/ui.rs`:
  - обновлён footer для viewer hotkeys (`F2`, `/`, `n/N`);
  - подсветка найденных строк и активного match;
  - расширен статус viewer (mode, offset, lines, bytes, matches).

## Checklist
[x] Добавить `hex-mode` для бинарных файлов и переключение режимов `text/hex`.
[x] Добавить поиск внутри viewer (`/`, next/prev, highlight match).
[x] Поддержать viewer для remote/SFTP файлов через ограниченный buffered read.
[x] Добавить расширенный статус viewer (mode, offset, line/byte position, matches).

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: добавлены viewer modes (`text/hex`) и hotkey `F2`.
- 2026-02-13: реализован in-view search (`/`, `n`, `N`) и подсветка совпадений.
- 2026-02-13: включён preview-read для local/sftp/archive через `read_file_preview`.
- 2026-02-13: обновлены `PLANS.md`, `README.md`.
- 2026-02-13: `cargo fmt`, `cargo test`, `cargo run -- --smoke` успешно.
