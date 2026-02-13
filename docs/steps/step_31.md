# Step 31: Find file через `fd` и panelized results

## Цель
Добавить быстрый поиск файлов на базе внешнего `fd` с асинхронным выполнением, live-индикацией прогресса и выдачей результатов в отдельном panelized-режиме.

## Решения
- Добавлен модуль `src/find.rs`:
  - парсинг find-ввода `pattern [--glob] [--hidden] [--follow]`;
  - проверка доступности `fd` в `PATH`;
  - асинхронный запуск `fd` в отдельном потоке с прогресс-ивентами;
  - сбор результатов в `FsEntry` через локальный `stat`.
- Расширена модель (`src/model.rs`):
  - `Event::Find(FindUpdate)`;
  - `FindRequest` и `FindUpdate::{Progress,Done,Failed}`;
  - `PanelState.find_view` для panelized find-режима;
  - `AppState.find_progress` для live-статуса в footer.
- Интеграция в `src/app.rs`:
  - `Left/Right -> Find (fd)` теперь рабочий action (без planned-заглушки);
  - диалог запуска find с fallback-ошибкой при отсутствии `fd`;
  - применение результатов в виртуальную панель (`..` + найденные элементы);
  - `Enter` по результату делает jump в исходный путь (`dir -> open`, `file -> parent+select`);
  - `..`/`Backspace` выходят из find-panelized режима обратно к обычному листингу.
- Обновлён UI (`src/ui.rs`):
  - заголовок панели показывает find-режим (`[fd:query:flags]`);
  - footer показывает live-статус поиска (`running/done`, matches, panel id).

## Checklist
[x] Добавить команду поиска файлов на базе внешнего `fd` (`name`, `glob`, optional hidden/follow symlinks).
[x] Реализовать асинхронный запуск `fd` через job/workers с live-progress в status/footer.
[x] Показать результаты в виртуальной панели (panelized view) с переходом к исходному пути.
[x] Сделать fallback: если `fd` не найден, показать понятный диалог с подсказкой по установке.

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: реализован async runner `fd` с progress/result/error ивентами.
- 2026-02-13: добавлен panelized find-view (`..` для выхода, jump по `Enter`).
- 2026-02-13: меню `Find (fd)` переведено из planned-заглушки в рабочий flow.
- 2026-02-13: обновлены `PLANS.md`, `README.md`.
- 2026-02-13: `cargo fmt`, `cargo test`, `cargo run -- --smoke` успешно.
