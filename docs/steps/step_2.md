# Step 2: Слой доменной модели и FS-адаптер

## Цель
Сформировать надежный слой работы с файловой системой (POSIX-first), чтобы дальнейшие шаги (UI, навигация, фоновые jobs) опирались на единый API и единый тип ошибок.

## Решения
- Добавить модуль `src/fs.rs` с `FsAdapter`.
- Реализовать операции:
  - `list_dir`
  - `stat_entry`
  - `create_dir`
  - `remove_path`
  - `move_path`
  - `copy_path`
- Ввести функции нормализации:
  - для существующих путей (absolute + canonical)
  - для новых путей (absolute + canonical parent)
- Вынести сортировку (name/size/mtime) и фильтрацию скрытых файлов в FS-слой.
- Расширить `AppError` до классификации `permission`, `not found`, `io` + контекст операции.

## Checklist
[x] Реализовать POSIX-ориентированный FS-адаптер (листинг, stat, mkdir, remove, rename/move, copy).
[x] Ввести нормализацию и валидацию путей (absolute/canonical где нужно).
[x] Добавить сортировки (name/size/mtime) и базовую фильтрацию скрытых файлов.
[x] Подготовить единый тип ошибок с понятной классификацией (permission, not found, io).

## Прогресс
- 2026-02-12: step создан.
- 2026-02-12: добавлен `FsAdapter` в `src/fs.rs` с API `list/stat/mkdir/remove/move/copy`.
- 2026-02-12: реализованы `normalize_existing_path`/`normalize_new_path` и резолв destination path для copy/move.
- 2026-02-12: добавлены сортировки (`Name/Size/ModifiedAt`) и фильтр скрытых файлов.
- 2026-02-12: `AppError` расширен контекстом операции и классификацией `permission/not found/io`.
- 2026-02-12: интегрирована загрузка панелей через FS-слой в `App::bootstrap` и `refresh`.
