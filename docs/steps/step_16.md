# Step 16: Полировка и валидация итерации 2

## Цель
Закрыть валидацию итерации 2: расширить smoke-проверки под batch-флоу, проверить стабильность layout и актуализировать пользовательскую документацию.

## Решения
- Расширен `--smoke` (`src/smoke.rs`):
  - добавлены batch-сценарии copy/move/delete через `WorkerPool`;
  - добавлен сбор метрик по batch-операциям;
  - добавлена валидация итогового состояния файловой системы после каждого batch-сценария.
- Добавлены unit-тесты стабильности layout (`src/ui.rs`):
  - проверка переключения режимов `Minimal/Compact/Full`;
  - проверка, что расчет ширин таблицы не превышает ширину панели.
- Обновлен `README.md`:
  - новые hotkeys (включая selection flow и `F10`);
  - новые dialog controls (`Tab/Shift+Tab`, `Left/Right`, `Enter`, `Esc`, `Alt+...`);
  - добавлен раздел `Known UX Constraints`;
  - обновлен дальнейший backlog.

## Checklist
[x] Smoke-сценарии для multi-select и batch copy/move/delete.
[x] Проверка стабильности layout на разных размерах терминала.
[x] Обновить README: новые hotkeys, selection flow, dialog controls.
[x] Добавить раздел Known UX constraints и дальнейший backlog (preview, tabs, bookmarks).

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: в smoke добавлены batch-метрики `batch_copy_total_ms`, `batch_move_total_ms`, `batch_delete_total_ms`.
- 2026-02-13: в smoke добавлена проверка консистентности результатов batch copy/move/delete.
- 2026-02-13: добавлены unit-тесты layout-расчета в `src/ui.rs`.
- 2026-02-13: обновлен `README.md` под актуальный функционал итерации 2.
- 2026-02-13: `cargo fmt`, `cargo test`, `cargo run -- --smoke` успешно.
