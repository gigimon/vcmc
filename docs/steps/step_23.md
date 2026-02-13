# Step 23: Footer-кнопки на всю ширину (adaptive MC style)

## Цель
Сделать нижнюю панель кнопок адаптивной и full-width, чтобы кнопочный footer оставался читаемым на разной ширине терминала и в разных режимах.

## Решения
- В `src/ui.rs` переработан `render_footer`:
  - footer рендерится как набор ячеек на всю ширину (`Mode + context buttons (+ status в Viewer)`),
  - ширина распределяется детерминированно между ячейками (`distribute_width`),
  - каждая ячейка текстово подгоняется под выделенную ширину (`fit_footer_cell_text`).
- Добавлен слой адаптации подписей:
  - `abbreviate_footer_label` для длинных лейблов (`Selection -> Select`, `Left/Right -> L/R`, ...),
  - безопасное `truncate` без "дрожания" геометрии.
- Обновлены context-наборы в новом формате:
  - `Normal`, `Selection`, `Dialog`, `Viewer`.
- Обновлены стили состояний:
  - `Mode`: контрастный бейдж;
  - `Button enabled`: яркий MC-like фон;
  - `Button disabled`: приглушенный стиль;
  - `Button active`: pressed-like акцент.
- Добавлены unit-тесты footer-геометрии:
  - сумма ширин ячеек всегда равна ширине footer;
  - текст в ячейке всегда укладывается ровно в выделенную ширину.

## Checklist
[x] Переработать footer в адаптивную full-width сетку с "кнопочным" видом.
[x] Добавить динамический layout для длинных подписей (`truncate`, `abbrev`) без дрожания геометрии.
[x] Обновить context sets (`Normal/Selection/Dialog/Viewer`) для новой сетки.
[x] Подсветить active/disabled/pressed состояния визуально ближе к MC.

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: `render_footer` переведен на full-width cell layout.
- 2026-02-13: добавлены label-abbrev и deterministic width distribution.
- 2026-02-13: добавлены unit-тесты footer layout/text fitting.
- 2026-02-13: `cargo fmt`, `cargo test`, `cargo run -- --smoke` успешно.
