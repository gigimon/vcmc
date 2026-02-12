# Step 3: Event loop и конкурентная модель

## Цель
Обеспечить неблокирующий UI при файловых операциях за счет отделения UI event loop от выполнения долгих FS-задач.

## Решения
- Оставить единый входной канал событий (`Event`) для:
  - пользовательского ввода (`Input`)
  - тиков (`Tick`)
  - resize (`Resize`)
  - ответов worker-пула (`Job` updates)
- Добавить `WorkerPool` в `src/jobs.rs`:
  - очередь `JobRequest` через `crossbeam-channel`
  - несколько worker-потоков
  - статусы `Running/Done/Failed` как `Event::Job`
- В `App`:
  - очередь задач `Copy/Move/Delete/Mkdir` с `JobStatus::Queued`
  - обработка прогресса и завершения jobs
  - перезагрузка панелей после успешных jobs

## Checklist
[x] Реализовать главный цикл обработки: input events, ticks, worker responses.
[x] Поднять worker pool для долгих операций (copy/move/delete) через каналы.
[x] Добавить очередь задач и модель прогресса (`Queued`, `Running`, `Done`, `Failed`).
[x] Гарантировать неизменность UI-потока: любые тяжелые операции только через воркеры.

## Прогресс
- 2026-02-12: step создан.
- 2026-02-12: добавлен `WorkerPool` (`src/jobs.rs`) и очередь `JobRequest`.
- 2026-02-12: `App` переведен на асинхронное выполнение `Copy/Move/Delete/Mkdir` через workers.
- 2026-02-12: обработка `Event::Job` обновляет прогресс jobs и перезагружает панели после `Done`.
- 2026-02-12: `cargo fmt` и `cargo check` проходят успешно.
