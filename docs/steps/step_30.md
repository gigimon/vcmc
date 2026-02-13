# Step 30: Архивы как VFS (zip/tar/tar.gz)

## Цель
Добавить archive panel-mode в стиле VFS: открывать архив как каталог, навигироваться по его структуре и копировать данные из архива в обычные backend-панели.

## Решения
- В `src/model.rs` добавлен новый backend-тип:
  - `BackendSpec::Archive(ArchiveConnectionInfo { archive_path })`.
- В `src/backend.rs` добавлен `ArchiveFsBackend` (read-only):
  - поддержка форматов `zip`, `tar`, `tar.gz`, `tgz`;
  - построение виртуального дерева (`/` + children) и `list/stat/read` поверх архива;
  - `create/remove/move/copy(write inside)/write` заблокированы как read-only операции;
  - helper `is_archive_file_path` для детекта поддерживаемых расширений.
- В `src/app.rs` добавлен flow archive mount/unmount:
  - `Enter` по архивному файлу на local-панели монтирует архив в активную панель;
  - `Backspace` в корне архива (`/`) размонтирует архив и возвращает локальную панель;
  - menu action `Left/Right -> Archive VFS` теперь рабочий (не planned-alert).
- Ограничения v1 archive mode в app-слое:
  - разрешены browse + copy out (`archive -> local/sftp`);
  - запрещены move/delete/mkdir внутри архива и copy into archive.
- В `src/ui.rs` добавлена явная индикация backend в title панели:
  - `local`, `sftp:user@host`, `archive:<name>`.

## Checklist
[x] Ввести `ArchiveFs` backend и подключение как panel-mode поверх существующей `FsBackend` абстракции.
[x] Реализовать вход в архив по `Enter` как в каталог и навигацию внутри архива.
[x] Поддержать операции v1: browse + copy out (`archive -> local/sftp`), copy in как отдельный этап.
[x] Добавить явную индикацию, что панель находится в архивном VFS-режиме.

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: добавлен `BackendSpec::Archive` и read-only `ArchiveFsBackend`.
- 2026-02-13: подключен mount/unmount archive flow (`Enter`, `Backspace` at `/`, menu action).
- 2026-02-13: включен copy out из archive VFS и блокировки unsupported write-операций.
- 2026-02-13: добавлена backend-индикация в panel title.
- 2026-02-13: обновлены `README.md` и `PLANS.md`.
- 2026-02-13: `cargo fmt`, `cargo test`, `cargo run -- --smoke` успешно.
