# Step 36: SFTP bookmarks

## Цель
Добавить повторно используемые SFTP-закладки с быстрым подключением и управлением из верхнего меню (`add/edit/delete/connect`), без ручного повторного ввода сервера/логина/пароля.

## Решения
- В `src/menu.rs` добавлены пункты для `Left/Right`:
  - `Bookmark Connect`
  - `Bookmark Add`
  - `Bookmark Edit`
  - `Bookmark Delete`
- В `src/app.rs` добавлен отдельный state-machine flow для bookmark-диалогов:
  - `PendingBookmark`, `BookmarkAction`, `BookmarkStage`, `SftpBookmark`;
  - сценарии `add/edit/delete/connect` с пошаговыми dialog prompt-ами.
- Реализовано хранение bookmarks в конфиге:
  - путь: `$XDG_CONFIG_HOME/vcmc/bookmarks.toml` или `~/.config/vcmc/bookmarks.toml`;
  - функции: `load_sftp_bookmarks()`, `save_sftp_bookmarks()`, `validate_bookmark()`.
- В SFTP connect flow добавлена авто-подстановка bookmark:
  - в диалоге `Connect SFTP` теперь поддерживается ввод `@bookmark_name`;
  - подключение выполняется через сохраненные параметры.
- Обновлена пользовательская документация:
  - `README.md` дописан раздел про bookmark workflow и формат `bookmarks.toml`.

## Checklist
[x] Добавить модель bookmark для SFTP (name/host/port/user/path/auth mode).
[x] Добавить хранение bookmarks в конфиге (`XDG config`/`~/.config/vcmc`) с загрузкой при старте.
[x] Реализовать меню bookmarks: быстрый connect, добавить, изменить, удалить.
[x] Поддержать авто-подстановку bookmark в flow подключения (`Left/Right -> Connect SFTP`).

## Прогресс
- 2026-02-13: step создан.
- 2026-02-13: реализованы `Bookmark Add/Edit/Delete/Connect` в top menu.
- 2026-02-13: добавлено хранение в `bookmarks.toml` и подключение через `@bookmark_name`.
- 2026-02-13: обновлены `README.md` и `PLANS.md`.
