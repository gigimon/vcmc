# Step 20: Интеграция внешнего редактора (`F4`)

## Цель
Реализовать безопасный запуск внешнего редактора через `$EDITOR` с корректным выходом/возвратом в TUI и обновлением панелей после редактирования.

## Решения
- Добавлена команда `OpenEditor` (`F4`) в `src/model.rs` и keymap в `src/app.rs`.
- Реализован `App::open_editor`:
  - проверка, что выбран не виртуальный элемент и не директория;
  - fallback при отсутствии `$EDITOR` (`alert` с подсказкой `export EDITOR='nvim'`);
  - запуск редактора для текущего файла через `sh -lc "<EDITOR> <path>"`.
- Добавлены terminal helpers в `src/terminal.rs`:
  - `suspend_for_external_process()` (disable raw mode + leave alternate screen);
  - `resume_after_external_process()` (enter alternate screen + enable raw mode).
- Чтобы input thread не перехватывал ввод редактора, добавлена пауза в `src/runtime.rs`:
  - `set_input_poll_paused(bool)` + проверка в `spawn_event_pump`.
- После завершения редактора:
  - восстанавливается TUI;
  - выполняется refresh обеих панелей (`mtime/size/name` обновляются);
  - пишется статус в activity log.
- Footer теперь показывает `F4 Edit` как доступный для редактируемых текущих файлов.

## Checklist
[x] Реализовать запуск `$EDITOR <current_file>` с временным выходом из alternate screen.
[x] После завершения editor корректно вернуть raw mode/alternate screen и redraw.
[x] Добавить fallback, если `$EDITOR` не задан (alert + подсказка).
[x] Обновлять панель после возврата из editor (mtime/size/имя).

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: добавлен `F4` keymap и команда `OpenEditor`.
- 2026-02-13: реализованы terminal suspend/resume helpers.
- 2026-02-13: добавлена пауза event-pump на время внешнего редактора.
- 2026-02-13: добавлен refresh панелей после закрытия редактора.
- 2026-02-13: `cargo fmt` и `cargo test` успешно.
