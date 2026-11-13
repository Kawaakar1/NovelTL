//! Режим «Буфер обмена» (все ОС): текст для перевода берётся из буфера обмена.
//! Подходит для игр, которые не берёт LunaHook, и для Linux, где LunaHook нет: игра идёт через
//! Wine/Proton, а программа-перехватчик (например, Textractor) копирует реплики в буфер.
//!
//! В списке окон это виртуальное «окно» с особыми номерами; мастер, профиль, кэш и плашка
//! работают с ним так же, как с игрой.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use noveltl_core::i18n::{t, tf};

use crate::source::{HookEvent, HookSource, ThreadInfo, Waker};
use crate::winutil::WindowInfo;

/// Номер «окна» и «процесса» буфера обмена (настоящие такими не бывают).
pub const CLIPBOARD_HWND: isize = -100;
pub const CLIPBOARD_PID: u32 = u32::MAX - 1;
/// «Путь к exe» для профиля: по нему профиль буфера обмена находится, как профиль игры.
pub const CLIPBOARD_EXE: &str = "clipboard";
const POLL: Duration = Duration::from_millis(300);
/// Длинный текст — скорее всего, скопирован не из игры.
const MAX_CHARS: usize = 1500;

pub fn window_info() -> WindowInfo {
    WindowInfo {
        hwnd: CLIPBOARD_HWND,
        pid: CLIPBOARD_PID,
        title: t("Буфер обмена (скопированный текст)").to_string(),
        exe_path: CLIPBOARD_EXE.to_string(),
    }
}

fn thread() -> ThreadInfo {
    ThreadInfo { code: "CLIPBOARD".into(), name: t("Буфер обмена").to_string(), pid: CLIPBOARD_PID, ..Default::default() }
}

/// Новый текст из буфера, который стоит переводить.
fn accept(text: &str, last: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && trimmed != last.trim() && trimmed.chars().count() <= MAX_CHARS
}

/// Текст, который сама программа положила в буфер (кнопка «копировать» на плашке): один раз не переводим.
static IGNORED: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Запомнить текст, который не надо переводить при первом появлении в буфере.
pub fn ignore_next(text: &str) {
    *IGNORED.lock().unwrap_or_else(|e| e.into_inner()) = text.trim().to_string();
}

/// true, если это запомненный текст (запись при этом сбрасывается).
fn take_ignored(text: &str) -> bool {
    let mut g = IGNORED.lock().unwrap_or_else(|e| e.into_inner());
    let hit = !g.is_empty() && g.as_str() == text.trim();
    if hit {
        g.clear();
    }
    hit
}

/// Кладёт текст в буфер обмена так, чтобы режим «Буфер обмена» не перевёл его снова.
pub fn copy_text(text: &str) -> bool {
    ignore_next(text);
    arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())).is_ok()
}

/// Основной источник (LunaHook / демо) плюс буфер обмена для `CLIPBOARD_PID`.
pub struct WithClipboard {
    inner: Box<dyn HookSource>,
    events: Sender<HookEvent>,
    waker: Waker,
    stop: Option<Arc<AtomicBool>>,
}

impl WithClipboard {
    pub fn new(inner: Box<dyn HookSource>, events: Sender<HookEvent>, waker: Waker) -> WithClipboard {
        WithClipboard { inner, events, waker, stop: None }
    }

    fn start(&mut self) {
        self.stop_watch();
        let stop = Arc::new(AtomicBool::new(false));
        self.stop = Some(stop.clone());
        let (tx, waker) = (self.events.clone(), self.waker.clone());
        let _ = std::thread::Builder::new().name("clipboard".into()).spawn(move || {
            let send = |ev: HookEvent| {
                let _ = tx.send(ev);
                waker();
            };
            let mut cb = match arboard::Clipboard::new() {
                Ok(c) => c,
                Err(e) => {
                    send(HookEvent::AttachFailed(CLIPBOARD_PID, tf("Нет доступа к буферу обмена: {}", &[&e])));
                    return;
                }
            };
            // то, что лежало в буфере до начала, не переводим
            let mut last = cb.get_text().unwrap_or_default();
            send(HookEvent::Connected(CLIPBOARD_PID));
            send(HookEvent::ThreadCreated(thread()));
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(POLL);
                if let Ok(text) = cb.get_text() {
                    if !take_ignored(&text) && accept(&text, &last) {
                        send(HookEvent::Text(thread(), text.trim().to_string()));
                    }
                    last = text;
                }
            }
        });
    }

    fn stop_watch(&mut self) {
        if let Some(s) = self.stop.take() {
            s.store(true, Ordering::SeqCst);
        }
    }
}

impl HookSource for WithClipboard {
    fn attach(&mut self, pid: u32, codepage: u32) {
        if pid == CLIPBOARD_PID {
            self.start();
        } else {
            self.inner.attach(pid, codepage);
        }
    }

    fn detach(&mut self, pid: u32) {
        if pid == CLIPBOARD_PID {
            self.stop_watch();
        } else {
            self.inner.detach(pid);
        }
    }

    fn shutdown(&mut self) {
        self.stop_watch();
        self.inner.shutdown();
    }

    fn error(&self) -> String {
        self.inner.error()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_new_short_text() {
        assert!(accept("「おはよう」", ""));
        assert!(!accept("「おはよう」", "「おはよう」\n"));
        assert!(!accept("   ", ""));
        assert!(!accept(&"あ".repeat(MAX_CHARS + 1), ""));
    }

    #[test]
    fn ignored_text_is_skipped_once() {
        ignore_next("Привет");
        assert!(take_ignored(" Привет "));
        assert!(!take_ignored("Привет"));
        assert!(accept("Привет", ""));
    }
}
