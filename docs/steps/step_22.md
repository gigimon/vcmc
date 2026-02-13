# Step 22: MC-like progress UI для batch операций

## Цель
Показать в интерфейсе агрегированный прогресс batch-операций в стиле MC (`N/M` + текущий файл), не блокируя UI.

## Решения
- Расширена модель job/update:
  - в `JobUpdate` добавлены поля `current_item`, `batch_completed`, `batch_total`;
  - в `Job` добавлены те же поля для хранения агрегированных статусов;
  - в `AppState` добавлен `batch_progress: Option<BatchProgressState>`.
- В worker-слое (`src/jobs.rs`) каждое событие `Running/Done/Failed` теперь передает `current_item` (имя файла).
- В app-агрегаторе (`src/app.rs`):
  - `BatchProgress` расширен полем `current_file`;
  - при batch-update вычисляется и прокидывается прогресс (`completed/total`) в `JobUpdate`;
  - поддерживается live snapshot в `state.batch_progress`;
  - добавлен сброс visible progress при завершении batch и при ошибке постановки задач.
- В UI (`src/ui.rs`) добавлен progress overlay:
  - `Operation`,
  - `Current file`,
  - `Files: N/M` + `failed`,
  - текстовый progress-bar с процентом.

## Checklist
[x] Расширить `JobUpdate`/batch агрегатор: передавать текущий файл и прогресс по элементам пачки.
[x] Добавить прогресс-панель/оверлей в стиле MC: `Operation`, `Current file`, `Files: N/M`.
[x] Для delete/copy/move показывать общий прогресс и текущий элемент в реальном времени.
[x] Сохранить неблокирующий UI и корректное завершение/сброс состояния progress.

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: добавлены batch-progress поля в `JobUpdate`/`Job`/`AppState`.
- 2026-02-13: реализован live batch-агрегатор с `current_file` и `completed/total`.
- 2026-02-13: добавлен MC-like progress overlay.
- 2026-02-13: `cargo fmt`, `cargo test`, `cargo run -- --smoke` успешно.
