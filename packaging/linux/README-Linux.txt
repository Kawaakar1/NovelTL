NovelTL для Linux
=================

Запуск: распакуйте архив в любую папку и запустите ./noveltl
Добавить в меню приложений: ./install.sh
Настройки, профили и кэш хранятся рядом с программой, в папке data.

ЧЕМ ОТЛИЧАЕТСЯ ОТ WINDOWS
- Автоматического перехвата текста (LunaHook) в Linux нет — он существует только для Windows.
  Текст берётся из буфера обмена: в мастере выберите «Буфер обмена (скопированный текст)».
  Обычно игру запускают через Wine/Proton, а программа-перехватчик (например, Textractor,
  запущенный в том же Wine) копирует каждую реплику в буфер — NovelTL сразу её переводит.
  Подойдёт и любой другой способ скопировать текст (Ctrl+C из текстового окна игры и т. п.).
- Плашка — отдельное прозрачное окно поверх всех окон. Откройте замочек на плашке, перетащите
  её на окно диалога и растяните за края, затем закройте замочек — клики снова идут в игру.
- Значка в трее и глобальных горячих клавиш нет: всё есть в меню ☰ на плашке.
- Выбор аватара — только PNG; выбор сетевого интерфейса — только в Windows.
- Ключ DeepL и пароль прокси в Linux хранятся в data/settings.json без шифрования.
- Плашка работает в X11 (в том числе XWayland, который NovelTL выбирает сам).
  В чистом Wayland окно не может само встать в нужное место и поверх игры.
  Steam Deck: работает в режиме рабочего стола; в игровом режиме (Gaming Mode)
  окна других программ поверх игры не показываются.
- Для японского, китайского и корейского нужен шрифт Noto Sans CJK
  (пакет fonts-noto-cjk / noto-fonts-cjk / google-noto-sans-cjk-fonts).

Сборка для Linux проверяется автоматически на GitHub, но с настоящими играми на Linux
её пока никто не проверял — сообщайте о проблемах в Issues:
https://github.com/Kawaakar1/NovelTL/issues


NovelTL for Linux
=================

Run: extract the archive anywhere and start ./noveltl
Add to the application menu: ./install.sh
Settings, profiles and cache are stored next to the program, in the data folder.

DIFFERENCES FROM WINDOWS
- There is no automatic text capture (LunaHook) on Linux — it exists only for Windows.
  Text comes from the clipboard: in the wizard choose "Clipboard (copied text)".
  Usually the game runs through Wine/Proton and a text hooker (for example Textractor running
  in the same Wine) copies every line to the clipboard — NovelTL translates it right away.
- The overlay is a separate transparent always-on-top window. Open the lock on the overlay,
  drag it onto the dialogue box and stretch it by the edges, then close the lock.
- No tray icon and no global hotkeys: everything is in the ☰ menu on the overlay.
- Avatar: PNG only; choosing a network interface — Windows only.
- On Linux the DeepL key and proxy password are stored in data/settings.json unencrypted.
- The overlay works on X11 (including XWayland, which NovelTL selects automatically).
  On pure Wayland a window cannot place itself or stay above the game.
  Steam Deck: works in Desktop Mode; Gaming Mode does not show other windows over the game.
- Japanese, Chinese and Korean need the Noto Sans CJK font
  (package fonts-noto-cjk / noto-fonts-cjk / google-noto-sans-cjk-fonts).

The Linux build is checked automatically on GitHub, but nobody has tried it with real games
on Linux yet — please report problems in Issues:
https://github.com/Kawaakar1/NovelTL/issues
