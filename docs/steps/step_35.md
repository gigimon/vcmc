# Step 35: Поиск по содержимому (content search)

## Цель
Добавить быстрый поиск по содержимому на базе `rg` с panelized-результатами и безопасным UX (fallback при отсутствии бинарника + отмена поиска).

## Решения
- В `src/model.rs` расширена модель поиска:
  - `FindKind` (`NameFd`, `ContentRg`);
  - `FindRequest`, `FindUpdate`, `FindPanelState`, `FindProgressState` получили поля для `rg`-режима (`glob_pattern`, `case_sensitive`, `kind`) и событие `Canceled`.
- В `src/find.rs` добавлены:
  - `is_rg_available()`;
  - `parse_content_search_input()` для `pattern [--glob GLOB] [--hidden] [--case-sensitive|--ignore-case]`;
  - `spawn_rg_search()` с асинхронным запуском `rg --vimgrep`, live-progress и парсингом `file:line:snippet`;
  - отмена активного поиска через `cancel_running_find(id)` (kill процесса + canceled state).
- В `src/app.rs`:
  - добавлен новый prompt `Search text` и запуск на локальной панели;
  - добавлена обработка `Esc` для отмены активного поиска;
  - результаты `rg` попадают в panelized view и открываются через текущий jump-flow;
  - fallback-диалог при отсутствии `rg`.
- В `src/menu.rs` добавлены пункты `Search files` и `Search text` для `Left/Right`.
- В `src/ui.rs`:
  - find-badge и footer status теперь различают `fd`/`rg` и показывают режим/флаги.
- В `README.md` добавлена документация по workflow `Search text` и обновлены ограничения.

## Checklist
[x] Добавить поиск по содержимому на базе `rg` (`pattern`, optional `glob`, `case`, `hidden`).
[x] Реализовать асинхронный запуск `rg` с live-прогрессом и отменой.
[x] Показать результаты в panelized view: `file:line:snippet` с переходом в файл/директорию.
[x] Добавить fallback-диалог, если `rg` отсутствует в системе.

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: реализован `rg` content search + panelized view + fallback.
- 2026-02-13: добавлена отмена активного поиска (`Esc`) и `Canceled` flow.
- 2026-02-13: обновлены `README.md` и `PLANS.md`.
