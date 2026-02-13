# Step 29: Матрица конфликтов copy/move

## Цель
Убрать `abort-only` поведение при конфликтах destination для copy/move и добавить интерактивный conflict-flow в стиле MC: выбор действия для каждого конфликта с поддержкой `apply to all`.

## Решения
- В `src/app.rs` добавлено состояние `PendingConflict`:
  - пошаговый разбор конфликтов перед постановкой job;
  - накопление резолвнутых элементов (`ready`) и статистики `skipped`;
  - глобальная политика `apply_all` (`Overwrite`, `Skip`, `OverwriteIfNewer`).
- Добавлен conflict-dialog с действиями:
  - `Overwrite`, `Skip`, `Rename` (auto-rename `_copyN`), `Newer`;
  - `OverAll`, `SkipAll`, `NewerAll`, `Cancel`.
- Для single copy/move (`F5/F6` через rename prompt) и batch copy/move (`selection`) включен единый conflict-flow.
- Batch preflight больше не валится на `destination already exists`; конфликты решаются интерактивно.
- В conflict body добавлены подсказки по `size/mtime` для source/target и hint по "newer/older".
- Для overwrite-ветки добавлен pre-remove destination перед enqueue job (с обработкой ошибок и skip при невозможности).
- Batch queue/log/progress обновлены:
  - в summary логируется число `skipped`;
  - прогресс `N/M` строится по реально queued элементам после conflict-resolution.

## Checklist
[x] Добавить интерактивный conflict-dialog: `Overwrite`, `Skip`, `Rename` + `Apply to all`.
[x] Поддержать стратегии `newer-only` и `size/mtime`-подсказки в теле диалога.
[x] Интегрировать conflict-policy в batch pipeline и прогресс (`N/M`, failed/skipped).
[x] Обновить обработку ошибок и activity log для прозрачного post-mortem.

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: реализован conflict state machine и диалог с multi-action.
- 2026-02-13: включен `newer-only` policy для single и batch.
- 2026-02-13: добавлен auto-rename fallback (`_copyN`) при конфликте.
- 2026-02-13: обновлены batch queue/progress/log с учетом skipped.
- 2026-02-13: `cargo fmt`, `cargo test`, `cargo run -- --smoke` успешно.
