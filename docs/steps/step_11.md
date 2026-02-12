# Step 11: Selection Engine (MC-like)

## Цель
Добавить полноценную модель выбора файлов в панели в стиле MC для дальнейших batch-операций.

## Решения
- Выбор хранится на уровне `PanelState` как `selected_paths` и не ломается при сортировке/фильтре.
- Реализованы хоткеи:
  - `Space`/`Ins` — toggle текущего элемента,
  - `+` — select by mask,
  - `-` — deselect by mask,
  - `*` — invert selection,
  - `Shift+Up/Down` — диапазон с anchor.
- Добавлен mask-диалог с wildcard-маской (`*`, `?`).
- Добавлена визуализация выбранных строк и счетчик выбранных элементов/объема.

## Checklist
[x] Добавить в `PanelState` модель multi-select (`selected_paths`, `selection_anchor`).
[x] Реализовать `Space/Ins` (toggle current), `+` (select by mask), `-` (deselect by mask), `*` (invert).
[x] Реализовать диапазонное выделение (`Shift+Up/Down`) с anchor-позицией.
[x] В status bar показывать счетчик выделенных (`selected N items / bytes`).

## Прогресс
- 2026-02-12: step создан.
- 2026-02-12: добавлены `selected_paths` и `selection_anchor` в `PanelState`.
- 2026-02-12: реализованы команды выделения (`Space/Ins`, `+`, `-`, `*`, `Shift+Up/Down`).
- 2026-02-12: добавлен mask input-диалог и wildcard matching (`*`, `?`).
- 2026-02-12: UI подсвечивает выбранные строки и показывает `sel:N (size)` в status bar.
