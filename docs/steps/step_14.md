# Step 14: UI Kit для диалогов с кнопками

## Цель
Сделать все модальные окна единообразными по внешнему виду и управлению, с фокусируемыми кнопками и клавиатурной навигацией в стиле MC.

## Решения
- Введена общая модель диалога в `model.rs`:
  - `DialogState` (`title/body/input_value/buttons/focused_button/tone`)
  - `DialogButton` + `DialogButtonRole`
- В `AppState` заменены разрозненные prompt-поля на единый `dialog`.
- В `app.rs` реализован единый обработчик диалогового ввода:
  - навигация кнопок: `Tab/Shift+Tab`, `Left/Right`
  - активация: `Enter`
  - отмена: `Esc`
  - ускорители: `Alt+буква` по `accelerator` кнопки
- В новый компонент переведены все текущие диалоги:
  - confirm (включая batch confirm)
  - alert
  - rename/copy-as/move-as
  - mask select/deselect
- В `ui.rs` удалены отдельные рендеры dialog-типов и добавлен единый `render_dialog` с footer-кнопками и highlighted focus.

## Checklist
[x] Ввести общий компонент диалога: title/body/footer buttons/focused button.
[x] Реализовать навигацию по кнопкам: `Tab/Shift+Tab`, `Left/Right`, `Enter`, `Esc`.
[x] Перевести текущие confirm/alert/rename в новый компонент с кнопками.
[x] Поддержать ускорители кнопок (`Alt+буква`) для частых действий.

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: добавлен единый `DialogState` и кнопочная модель в `AppState`.
- 2026-02-13: реализован общий keyboard flow для всех диалогов с фокусом кнопок.
- 2026-02-13: confirm/alert/rename/mask переведены на единый рендер `render_dialog`.
- 2026-02-13: поддержаны акселераторы `Alt+буква` для кнопок (`Yes/No/Apply/Cancel/OK`).
