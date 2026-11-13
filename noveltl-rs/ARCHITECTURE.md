# NovelTL на Rust — устройство программы (пакет `noveltl`)

Программа перенесена с Python-версии; колонка «Аналог в Python» и пометки «перенос …py» в коде указывают на её файлы. Python-версия удалена из репозитория и осталась в истории git (последний коммит с ней — `ae7a790`, версия 0.4.3). Интерфейс — на 5 языках (см. «Язык интерфейса»).

## Сборка пакета `noveltl`

```powershell
. .\env.ps1
$env:CARGO_TARGET_DIR = "$env:LOCALAPPDATA\noveltl-rs-target-app"   # отдельно от сборки ядра
cargo build --release -p noveltl
```

## Модули (`noveltl\src\`)

| Файл | Что делает | Аналог в Python |
|---|---|---|
| `main.rs` | разбор аргументов (`--demo`, `--shots ПАПКА`, `--selfcheck`, `--theme`), один экземпляр (мьютекс `NovelTL_Running`), шрифты egui, запуск `eframe` | `main.py` |
| `paths.rs` | папка программы (рядом с exe), `data`, `files`, подпапки; в демо — `data\demo` | `paths.py` |
| `overlay.rs` | **готово** — плашка: своё окно Windows + Direct2D/DirectWrite, свой поток; `Overlay::start()`, `send(Cmd)`, события `Event::Moved` | `ui/overlay.py` |
| `winutil.rs` | окна, процессы, запуск скрытый/с правами, сетевые интерфейсы | `winutil.py` |
| `source.rs` | события источника текста `HookEvent` и общий интерфейс для LunaHook и демо | `HookSource` |
| `lunahost.rs` | LunaHost64.dll через FFI, колбэки -> `HookEvent` в канал | `lunahost.py` |
| `demo.rs` | демо-источник и окно «игры» | `demo.py` |
| `hotkeys.rs` | глобальные клавиши (RegisterHotKey в своём потоке со скрытым окном) | `hotkeys.py` |
| `tray.rs` | значок в трее с меню | часть `main_window.py` |
| `controller.rs` | сессия игры, автоподбор, перевод в фоновом потоке, отчёт | `controller.py` |
| `ui/theme.rs` | тёмная/светлая палитра egui, «как в Windows» | `ui/theme.py` |
| `ui/main_window.rs`, `ui/wizard.rs`, `ui/settings.rs`, `ui/glossary.rs` | окна | `ui/*.py` |

## Потоки и события

- **Контроллер живёт в своём фоновом потоке** («движок»): `Arc<Mutex<Controller>>`, поток каждые ~30 мс вызывает
  `controller.poll()`. Причина (проверено): когда главное окно спрятано в трей, eframe НЕ вызывает `update()` —
  а перевод должен идти и в это время. Интерфейс берёт замок только на короткое время: прочитать состояние
  для отрисовки или вызвать метод контроллера. Нельзя держать замок, пока ждёшь чего-то долгого.
- **Показ окна из трея** — НЕ через `ViewportCommand::Visible(true)` (у спрятанного окна egui команды не обрабатывает),
  а напрямую: `ShowWindow(hwnd, SW_SHOW)` + `SetForegroundWindow(hwnd)` в обработчике события трея; после этого egui
  оживает сам. Прятать окно в трей можно через `ViewportCommand::Visible(false)`.
- Главный поток — egui (`eframe`): только отрисовка окон и вызовы методов контроллера.
- Фоновые потоки (LunaHook, переводчик, горячие клавиши, плашка) шлют события в `std::sync::mpsc` и будят интерфейс
  `egui::Context::request_repaint()` (Context можно клонировать и передавать в потоки).
- Колбэки LunaHook приходят из чужих потоков; в них только упаковать данные и отправить в канал.
- 64-битные адреса — `u64`, окна — `isize` (сырое значение HWND).

## Источник текста

```rust
pub enum HookEvent {
    Connected(u32), Disconnected(u32),
    ThreadCreated(ThreadInfo), ThreadRemoved(ThreadInfo),
    Text(ThreadInfo, String),
    HostInfo(i32, String),          // 1 — предупреждение, 4 — тип движка
    AttachFailed(u32, String),
}
pub struct ThreadInfo { pub code: String, pub name: String, pub pid: u32, pub addr: u64, pub ctx: u64, pub ctx2: u64 }
pub trait HookSource: Send {
    fn attach(&mut self, pid: u32, codepage: u32);
    fn detach(&mut self, pid: u32);
    fn insert_hook(&mut self, pid: u32, code: &str);
    fn shutdown(&mut self);
    fn error(&self) -> String;     // "" если всё хорошо
}
```

## Ядро (`noveltl-core`)

`cleaner`, `langdetect`, `scorer`, `profiles`, `cache`, `translators`, `pipeline`, `net` — см. `PORTING.md`.

### Язык интерфейса (`i18n`)

Русская строка в коде — ключ: `t("Настройки")`, с подстановками — `tf("Игра «{}» найдена…", &[&name])`.
Переводы на English, 日本語, 中文, 한국어 лежат в таблицах `i18n/app.rs` (окна, меню, плашка) и
`i18n/core.rs` (сообщения ядра). Нет перевода — показывается русская строка.
Язык выбирается в «Настройки» → «Общие» (`AppSettings::ui_lang`; не выбран — по языку Windows).
Новую строку интерфейса оборачивайте в `t(...)` и добавляйте в таблицу: тест `every_key_is_translated`
находит в исходниках все вызовы `t`/`tf` и падает, если у какой-то строки нет перевода на все языки.
