# Step 25: Backend-абстракция и нативный SFTP backend

## Цель
Начать переход от POSIX-only FS-слоя к backend-модели (`LocalFs`/`SftpFs`) как базу для локально-удаленного workflow.

## Решения
- Добавлен `src/backend.rs`:
  - trait `FsBackend` (list/stat/create/remove/move/copy/normalize);
  - `LocalFsBackend` (обертка над существующим `FsAdapter`);
  - `SftpFsBackend` на `ssh2` (TCP+SSH auth+SFTP).
- В модель добавлены `BackendSpec` и `SftpConnectionInfo` (`host/user/port/path/auth`).
- `App` переведен на backend-абстракцию для панелей:
  - хранит `left_backend/right_backend` (`Arc<dyn FsBackend>`);
  - хранит `left_backend_spec/right_backend_spec`;
  - листинг/навигация/нормализация путей выполняются через backend.
- Worker-слой (`src/jobs.rs`) переведен на backend-интерфейс:
  - `JobRequest` теперь несет `source_backend/destination_backend`;
  - операции `copy/move/delete/mkdir` выполняются для выбранных backend-спеков;
  - для разных backend реализован generic recursive copy (`local<->sftp`, `sftp->sftp`).
- Добавлен UI-flow подключения:
  - `F9` (`ConnectSftp`) открывает dialog;
  - формат: `user@host:port/path auth=agent|password|key [password=.. key=/path passphrase=..]`;
  - `local` в этом диалоге возвращает панель обратно на локальный backend.

## Checklist
[x] Ввести `FsBackend` abstraction (`LocalFs`, `SftpFs`) для панелей и операций.
[x] Реализовать `SftpFs` на Rust-библиотеке (`ssh2`/эквивалент): list/stat/read/write/mkdir/remove/rename.
[x] Добавить UI-флоу подключения: host/user/port/path + auth (agent/key/password).
[x] Поддержать копирование `local <-> sftp` и `sftp -> sftp` через существующую job-модель.

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: введена backend-абстракция и переведены панели/операции на `FsBackend`.
- 2026-02-13: реализован `SftpFsBackend` на `ssh2`.
- 2026-02-13: добавлен `F9` flow для SFTP-подключения и переключения panel backend.
- 2026-02-13: включен cross-backend copy/move в job model (`local<->sftp`, `sftp->sftp`).
- 2026-02-13: `cargo fmt`, `cargo test`, `cargo run -- --smoke` успешно.
