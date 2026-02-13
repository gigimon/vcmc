# Step 17: Viewer Mode и доменная модель

## Цель
Ввести базовый fullscreen-режим `Viewer` в доменную модель и event loop без деградации текущего panel workflow.

## Решения
- В `src/model.rs` добавлены:
  - `ScreenMode` (`Normal`, `Viewer`);
  - `ViewerState` (`path`, `title`, `lines`, `scroll_offset`, `is_binary_like`, `byte_size`);
  - поля `screen_mode` и `viewer` в `AppState`;
  - команды `OpenViewer`, `CloseViewer`, `ViewerScrollUp`, `ViewerScrollDown`.
- В `src/app.rs` добавлены:
  - keymap для viewer-режима (`Esc/F3/q` закрытие, `Up/Down` скролл);
  - `F3` в normal keymap для открытия viewer;
  - методы `open_viewer`, `close_viewer`, `viewer_scroll_up`, `viewer_scroll_down`;
  - изоляция ввода viewer в `on_event`, чтобы `q` в viewer закрывал viewer, а не приложение.
- В `src/ui.rs` добавлены:
  - ветка fullscreen-рендера `render_viewer` при `screen_mode == Viewer`;
  - новый footer-режим `VIEWER` с подсказками управления;
  - включение `F3 View` в `Normal/Selection` при viewable текущем файле.
- Переходы `Normal -> Viewer -> Normal` не изменяют состояние панелей (`cwd`, выделения, позиция), т.к. panel-state не мутируется при viewer-операциях.

## Checklist
[x] Добавить режим экрана `Viewer` в `AppState` (normal/viewer) и структуру `ViewerState`.
[x] В `ViewerState` хранить: `path`, `title`, `lines`, `scroll_offset`, `is_binary_like`, `byte_size`.
[x] Ввести команды/события для `F3`, закрытия viewer (`Esc/F3/q`) и вертикального скролла.
[x] Гарантировать, что переход в viewer не ломает выделения/позицию в панели при возврате.

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: добавлены `ScreenMode`/`ViewerState` и viewer-команды в модель.
- 2026-02-13: интегрирован viewer flow в app-loop и keymap (`F3`, `Esc/F3/q`, `Up/Down`).
- 2026-02-13: добавлен базовый fullscreen viewer-рендер и footer-режим `VIEWER`.
- 2026-02-13: `cargo fmt` и `cargo test` успешно.
