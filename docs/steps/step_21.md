# Step 21: Полировка viewer/editor и документация

## Цель
Закрыть итерацию 3 через smoke-валидацию viewer/editor workflow и актуализировать документацию по новым режимам.

## Решения
- Расширен `--smoke` в `src/smoke.rs`:
  - добавлены viewer smoke-checks:
    - `viewer_text_mode_ok` (текстовый файл не определяется как binary-like),
    - `viewer_binary_mode_ok` (бинарный файл определяется как binary-like),
    - `viewer_scroll_probe_ok` (базовый сценарий scroll-переходов);
  - добавлен editor roundtrip probe:
    - запуск внешнего процесса на текущем файле (`true <file>` как безопасный smoke-probe),
    - метрика `editor_roundtrip_ms`.
- Обновлен `README.md`:
  - добавлены hotkeys `F3/F4` и секция viewer controls;
  - описаны ограничения smart fallback (`256 KB`, line clamp, binary heuristic);
  - добавлены ограничения по `$EDITOR`;
  - обновлен backlog (hex-mode, search in viewer, internal editor).

## Checklist
[x] Smoke-сценарий: `F3` на текстовом и binary-like файле + базовый скролл.
[x] Smoke-сценарий: `F4` roundtrip (запуск editor, возврат в TUI, refresh панели).
[x] Обновить README: хоткеи viewer/editor и ограничения smart fallback.
[x] Зафиксировать Known UX constraints и backlog следующего этапа (hex-mode, search in viewer, internal editor).

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: smoke-отчет расширен метриками/проверками viewer/editor.
- 2026-02-13: README обновлен под workflow `F3/F4`.
- 2026-02-13: `cargo fmt`, `cargo test`, `cargo run -- --smoke` успешно.
