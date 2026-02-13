# Step 26: Полировка сети, документация и smoke

## Цель
Добавить базовую сетевую устойчивость SFTP-операций, подготовить smoke-сценарии для remote mode и обновить документацию по remote workflow/ограничениям.

## Решения
- В `src/backend.rs` добавлены сетевые guard-rails:
  - retry подключения (`SFTP_CONNECT_ATTEMPTS=3`);
  - TCP read/write timeouts;
  - базовая классификация ошибок (`auth/network/perm/path`) в сообщениях connect-failure.
- В `src/smoke.rs` добавлен optional SFTP smoke:
  - запускается только при наличии `VCMC_SFTP_SMOKE_*` env;
  - проверяет базовые операции `list/write/read/delete` на тестовом хосте;
  - отражается в отчете `sftp_smoke_enabled/sftp_smoke_ok/sftp_smoke_total_ms`.
- Обновлен `README.md`:
  - добавлен раздел remote workflow (`F9` + `local<->sftp` flow);
  - добавлены security notes по auth-mode;
  - зафиксированы ограничения remote режима.

## Checklist
[x] Добавить retry/timeout и понятные сетевые ошибки (auth/network/perm/path).
[x] Подготовить smoke-сценарии для progress UI и базовых SFTP операций на тестовом хосте.
[x] Обновить README: remote workflow, security notes, `dircolors`, progress controls.
[x] Зафиксировать ограничения v1 remote режима и backlog (resume, parallel transfers, bookmarks for hosts).

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: внедрены retry/timeout + error-classification в SFTP connect path.
- 2026-02-13: добавлен optional SFTP smoke (env-driven).
- 2026-02-13: README расширен под remote workflow/security/limitations.
- 2026-02-13: `cargo fmt`, `cargo test`, `cargo run -- --smoke` успешно.
