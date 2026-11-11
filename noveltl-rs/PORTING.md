# Перенос ядра NovelTL с Python на Rust (`noveltl-core`)

Исходники Python — в соседней папке `..\noveltl\` (рабочая, проверенная версия), их тесты — в `..\tests\`.
Rust-версия должна вести себя ТАК ЖЕ: те же правила, те же числа в оценках, те же русские тексты сообщений.

## Сборка и тесты

```powershell
cd <корень noveltl-rs>
. .\env.ps1                      # Rust, MinGW, короткая папка сборки
cargo test -p noveltl-core       # только ядро
```

Не собирать пакет `noveltl` (окна) — им занимается руководитель. Не менять `env.ps1`, корневой `Cargo.toml`,
папку `noveltl\`. В `noveltl-core\Cargo.toml` можно добавлять зависимости (только чистый Rust или уже
используемые; без новых C-библиотек) — перечисли их в отчёте.

## Раскладка модулей (`noveltl-core\src\`)

| Rust | Python | Кто зависит |
|---|---|---|
| `langdetect.rs` | `noveltl/langdetect.py` | cleaner, scorer, pipeline |
| `cleaner.rs` | `noveltl/cleaner.py` | scorer, pipeline |
| `scorer.rs` | `noveltl/scorer.py` | profiles (тип `HookRef`) |
| `profiles.rs` | `noveltl/profiles.py` | — |
| `cache.rs` | `noveltl/cache.py` | translators |
| `translators.rs` | `noveltl/translators.py` | pipeline |
| `pipeline.rs` | `noveltl/pipeline.py` | — |
| `net.rs` | `noveltl/net.py` | — |

`lib.rs` уже объявляет все модули. Пока модуль не перенесён, его файл — пустая заглушка. Трогай только свои файлы.

## Общие правила

- Тесты — в конце своего файла: `#[cfg(test)] mod tests { ... }`. Перенеси ВСЕ случаи из соответствующего
  Python-теста (те же входы и ожидания) и добавь, если что-то специфично для Rust.
- Регулярки — крейт `regex` (в нём нет lookahead/backreference — переписывай логику кодом, если нужно).
- Работа со строками — по `char`, не по байтам (японский текст!). Длина строки = число `char`
  (как `len()` в Python).
- Время — секунды `f64` (как `time.time()`); функции, где Python принимает `now`, принимают `Option<f64>`.
- Никаких `unwrap()` на внешних данных (файлы, сеть, JSON) — ошибки возвращать или обходить, как в Python.
- Публичные типы, которые сохраняются в JSON, — с `serde` и ТЕМИ ЖЕ именами полей, что в Python
  (`#[serde(default)]` для совместимости со старыми файлами). Существующие `data\profiles\*.json`,
  `data\settings.json` и кэш `data\cache\*.sqlite` от Python-версии должны читаться Rust-версией.
- Комментарии — по-русски, коротко, только где неочевидно.
- `cargo test -p noveltl-core` — без ошибок и без предупреждений компилятора в твоих файлах.
