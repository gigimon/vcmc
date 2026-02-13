# Step 34: Полировка итерации 5

## Цель
Добавить smoke-сценарии для ключевых UX-функций итерации 5: conflict matrix, archive VFS copy-out, `fd` find, viewer search/hex и editor chooser.

## Решения
- Расширен `src/smoke.rs`:
  - добавлены отдельные пробы `run_conflict_matrix_probe`, `run_archive_vfs_probe`, `run_fd_find_probe`, `run_viewer_search_hex_probe`, `run_editor_chooser_probe`;
  - добавлены новые поля в `SmokeReport` и вывод в `SMOKE REPORT` для прозрачной диагностики Step 34;
  - `fd`-сценарий сделан условным (`fd_find_enabled`), чтобы smoke не падал на системах без `fd`, но проверял fallback-путь.
- Conflict matrix проверяется через `App::bootstrap` + симуляцию клавиш:
  - массовый copy с коллизиями;
  - действия `Rename` и `Skip`;
  - проверка итоговых файлов и содержимого.
- Archive VFS проверяется через backend/jobs:
  - browse `/` и `/docs`;
  - copy out `archive -> local` через `WorkerPool`.
- Viewer+ проверяется на text и hex поиске:
  - поиск по `needle` в text mode;
  - переход к следующему match;
  - переключение в hex mode и поиск hex-паттерна.
- Editor chooser проверяется через меню `Options -> Editor Settings`:
  - проверка открытия chooser-диалога и списка кандидатов;
  - либо проверка корректного error-диалога, если редакторы в `PATH` не найдены.

## Checklist
[x] Добавить smoke-сценарии: conflict-matrix, archive VFS browse/copy out, `fd` find, viewer search/hex, editor chooser.
[x] Обновить `SMOKE REPORT` новыми сигналами по Step 34.
[x] Отметить прогресс в `PLANS.md` и документации шага.

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: реализованы smoke-пробы для conflict matrix, archive VFS, `fd` find, viewer search/hex и editor chooser.
- 2026-02-13: обновлен `SMOKE REPORT` с флагами Step 34.
- 2026-02-13: обновлен `PLANS.md` (Step 34 отмечен выполненным).
