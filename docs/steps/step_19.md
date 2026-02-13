# Step 19: UI и управление fullscreen viewer (`F3`)

## Цель
Довести fullscreen viewer до рабочего keyboard-flow: расширить управление прокруткой, отобразить runtime-статус и обеспечить прозрачные ошибки загрузки.

## Решения
- В `src/app.rs` расширено управление viewer:
  - добавлены команды `ViewerPageUp`, `ViewerPageDown`, `ViewerTop`, `ViewerBottom`;
  - keymap viewer: `Up/Down`, `PgUp/PgDn`, `Home/End`, `Esc/F3` (и `q`) для выхода;
  - добавлены методы `viewer_page_up/down`, `viewer_top/bottom`;
  - page-step рассчитывается от высоты терминала.
- В `src/ui.rs` улучшен viewer UI:
  - заголовок viewer теперь показывает режим (`text` / `binary-like`);
  - footer в режиме `VIEWER` показывает статус:
    - `off` (scroll offset),
    - `lines` (total lines),
    - `bytes` (размер файла),
    - `mode` (text/binary-like);
  - footer hotkeys дополнен `PgUp/PgDn/Home/End`.
- Ошибки загрузки viewer (permission/not found/read failure) проходят через существующий путь:
  - `open_viewer -> load_viewer_state -> Result`;
  - `apply_command` перехватывает ошибку и вызывает `show_alert`.

## Checklist
[x] Реализовать fullscreen-рендер viewer с header/body/footer hotkeys.
[x] Управление: `Up/Down`, `PgUp/PgDn`, `Home/End`, `Esc/F3` для выхода.
[x] Поддержать статус внизу viewer (offset, total lines/bytes, mode text/binary-like).
[x] Добавить обработку ошибок загрузки (permission denied, not found, read failure).

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: добавлены page/home/end команды и keymap viewer.
- 2026-02-13: добавлен viewer runtime-status в footer (`off/lines/bytes/mode`).
- 2026-02-13: подтверждена обработка ошибок загрузки через alert-диалог.
- 2026-02-13: `cargo fmt` и `cargo test` успешно.
