# Step 24: Темы и подсветка расширений через `dircolors`

## Цель
Добавить поддержку `dircolors`-совместимой темы для type/extension-подсветки с безопасным fallback и runtime reload.

## Решения
- Добавлен модуль `src/theme.rs`:
  - парсинг токенов `DIR`, `LINK`, `EXEC`, `RESET`, `*.ext`;
  - поддержка `LS_COLORS` (если задан) как источник поверх файла;
  - fallback-тема по умолчанию при отсутствии/битом конфиге.
- Источники конфигурации:
  - `VCMC_DIRCOLORS_PATH` (явный путь для переопределения),
  - `~/.dir_colors`, `~/.dircolors`, `/etc/DIR_COLORS`, `/etc/dircolors`.
- Добавлен `is_executable` в `FsEntry`, вычисление в `fs.rs` через POSIX mode bits.
- Интеграция темы в UI:
  - `ui::render` принимает `DirColorsTheme`;
  - `type_style` строится через theme-mapping (`type + extension + exec`).
- Runtime reload:
  - команда `Refresh` (`r`) теперь перезагружает тему из окружения/файла и затем обновляет панели.
- Добавлены unit-тесты:
  - парсинг `dircolors`-токенов и extension-правил;
  - парсинг ANSI style-кодов.

## Checklist
[x] Добавить парсер `dircolors` (основные токены: `DIR`, `LINK`, `EXEC`, `*.ext`, `RESET`).
[x] Построить маппинг `dircolors -> ratatui::Style` с fallback-темой по умолчанию.
[x] Применить расширенную раскраску в таблице файлов (по типу и расширению).
[x] Добавить hot-reload или reload по команде для теста тем в runtime.

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: добавлен `src/theme.rs` с parser/fallback/LS_COLORS support.
- 2026-02-13: theme-интеграция в `App` и `UI` завершена.
- 2026-02-13: добавлено поле `is_executable` в `FsEntry`.
- 2026-02-13: `cargo fmt`, `cargo test`, `cargo run -- --smoke` успешно.
