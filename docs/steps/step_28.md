# Step 28: Верхнее дополнительное меню (MC-like menu bar)

## Цель
Добавить верхнее меню в стиле MC, чтобы собрать ключевые действия в единый слой навигации и вынести в него подключение SFTP и точки входа для новых функций итерации 5.

## Решения
- Добавлен новый модуль `src/menu.rs`:
  - описаны группы меню `Left | Options | Right`;
  - описаны menu actions и пункты внутри групп;
  - добавлен hotkey-резолвер по `Alt+буква` для прямого входа в нужную группу.
- В `src/model.rs` расширен `AppState`:
  - добавлен `TopMenuState` (`open`, `group_index`, `item_index`);
  - добавлена команда `OpenTopMenu`.
- В `src/app.rs` добавлена menu-логика:
  - обработка top-menu input (`F9`, `Alt+L/O/R`, стрелки, `Enter`, `Esc`);
  - выполнение menu actions через существующие команды/флоу (`copy/move/delete/mkdir`, `shell`, `command line`, `sort`, `refresh`);
  - `Left/Right -> Connect SFTP` вызывает текущий SFTP connect/disconnect flow для выбранной стороны;
  - `Find (fd)`, `Archive VFS`, `Viewer Modes`, `Editor Settings` добавлены как menu entry-points с явным planned-alert.
- В `src/ui.rs` добавлен новый верхний слой рендера:
  - top menu bar (1 строка);
  - выпадающий popup активной группы с выделением текущего пункта;
  - footer `F9` обновлен на `Menu`.
- `README.md` обновлен под новый UX top menu и SFTP-вход через `Left/Right`-группы.

## Checklist
[x] Добавить верхнюю строку меню с группами действий (`Left`, `Options`, `Right`).
[x] Вынести подключение SFTP и файловые действия внутрь `Left/Right` секций и перевести `F9` на открытие верхнего меню.
[x] Добавить в `Left/Right` входные точки для новых функций (Find, Archive VFS), а в `Options` для Viewer/Editor настроек.
[x] Реализовать клавиатурную навигацию меню (`F9/Alt+буква`, стрелки, Enter, Esc) без регрессий в основном режиме.

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: реализован top menu state + keyboard navigation.
- 2026-02-13: подключен menu render (bar + popup) и обновлен footer `F9 -> Menu`.
- 2026-02-13: `Connect SFTP` и file actions перенесены в `Left/Right` menu с секциями и разделителями.
- 2026-02-13: добавлены planned entry-points для `fd`, `Archive VFS`, `Viewer modes`, `Editor settings`.
- 2026-02-13: обновлены `PLANS.md` и `README.md`.
- 2026-02-13: `cargo fmt`, `cargo test`, `cargo run -- --smoke` успешно.
