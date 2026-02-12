# Step 1: Bootstrap и каркас проекта

## Цель
Поднять рабочий минимальный каркас приложения, который:
- корректно запускает TUI в alternate screen;
- безопасно восстанавливает терминал при штатном и аварийном выходе;
- задает базовые доменные сущности для дальнейших шагов (`AppState`, `PanelState`, `FsEntry`, `Command`, `Event`, `Job`).

## Решения
- Структура модулей:
  - `main.rs` - bootstrap приложения и основной цикл.
  - `terminal.rs` - инициализация/восстановление терминала, panic hook.
  - `model.rs` - доменные типы состояния и событий.
  - `runtime.rs` - event pump (input + tick).
  - `ui.rs` - базовый placeholder-рендер.
  - `errors.rs` - единый error type для доменного слоя.
- Каналы: `crossbeam-channel` для передачи событий от runtime в UI loop.
- Логирование: `tracing` + `tracing-subscriber`.

## Checklist
[x] Инициализировать `cargo`-проект (`bin`) и базовую структуру модулей.
[x] Подключить зависимости: `ratatui`, `crossterm`, `crossbeam-channel`, `walkdir`, `anyhow`, `thiserror`, `tracing`, `tracing-subscriber`.
[x] Описать базовые сущности: `AppState`, `PanelState`, `FsEntry`, `Command`, `Event`, `Job`.
[x] Добавить graceful shutdown и восстановление терминала при panic/ошибке.

## Прогресс
- 2026-02-12: создан базовый `cargo`-проект.
- 2026-02-12: добавлены модульный каркас, базовый event pump, placeholder UI, panic hook и `TerminalGuard`.
- 2026-02-12: `cargo fmt` и `cargo check` выполнены успешно.
