# Step 33: Editor Discovery и first-run chooser для `F4`

## Цель
Снять зависимость `F4` от обязательного `$EDITOR`: добавить автообнаружение популярных редакторов, first-run выбор и сохранение настройки между сессиями.

## Решения
- В `src/app.rs` добавлен editor chooser flow:
  - `F4`: порядок выбора редактора: `$EDITOR` -> сохранённая настройка -> first-run chooser;
  - chooser показывает найденные варианты и принимает номер в диалоге;
  - выбор сохраняется в конфиг и может сразу использоваться для открытия файла.
- В `src/app.rs` реализовано автообнаружение editor-команд в `PATH`:
  - `nvim`, `vim`, `nano`, `hx`, `micro`, `emacs`, `code -w`.
- Добавлено сохранение/чтение editor-конфига:
  - путь: `$XDG_CONFIG_HOME/vcmc/config.toml` или `~/.config/vcmc/config.toml`;
  - ключ: `editor = "..."`.
- `Options -> Editor Settings` (top menu) теперь рабочий пункт:
  - позволяет сменить сохранённый editor без перезапуска.

## Checklist
[x] При отсутствии `$EDITOR` запускать автообнаружение популярных редакторов (`nvim`, `vim`, `nano`, `hx`, `micro`, `emacs`, `code -w`).
[x] Показать first-run диалог выбора редактора из найденных вариантов.
[x] Сохранять выбор в конфиг (`XDG config`/`~/.config/vcmc`) и использовать как default для `F4`.
[x] Добавить пункт в верхнее меню для смены редактора без перезапуска.

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: реализованы editor auto-discovery и first-run chooser для `F4`.
- 2026-02-13: добавлено сохранение editor-настройки в config (`XDG`/`~/.config/vcmc`).
- 2026-02-13: `Options -> Editor Settings` переведён из planned-заглушки в рабочий action.
- 2026-02-13: обновлены `PLANS.md`, `README.md`.
- 2026-02-13: `cargo fmt`, `cargo test`, `cargo run -- --smoke` успешно.
