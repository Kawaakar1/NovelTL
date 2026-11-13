//! Сторож зависаний: если окно перестало отвечать, пишет отчёт в data\reports\hang_*.txt —
//! кто держит замок контроллера (поток и место в коде), кто его ждёт и чем был занят интерфейс.

use std::collections::HashMap;
use std::panic::Location;
use std::sync::{Arc, Mutex, MutexGuard};
#[cfg(windows)]
use std::time::Duration;
use std::time::Instant;

#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, SMTO_ABORTIFHUNG, SMTO_BLOCK, WM_NULL};

struct State {
    /// кто держит замок контроллера: (поток, место в коде, с какого момента)
    holder: Option<(String, &'static Location<'static>, Instant)>,
    /// кто ждёт замок: поток -> (место в коде, с какого момента)
    waiting: HashMap<String, (&'static Location<'static>, Instant)>,
    /// чем занят интерфейс в текущем кадре
    ui_phase: &'static str,
    ui_phase_since: Option<Instant>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = g.get_or_insert_with(|| State { holder: None, waiting: HashMap::new(), ui_phase: "", ui_phase_since: None });
    f(s)
}

fn thread_name() -> String {
    let t = std::thread::current();
    t.name().map(str::to_string).unwrap_or_else(|| format!("{:?}", t.id()))
}

/// Замок, который помнит, кто и где его взял.
pub struct Tracked<'a, T> {
    guard: MutexGuard<'a, T>,
}

impl<T> std::ops::Deref for Tracked<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> std::ops::DerefMut for Tracked<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> Drop for Tracked<'_, T> {
    fn drop(&mut self) {
        with_state(|s| s.holder = None);
    }
}

/// Взять замок с учётом: место вызова видно в отчёте о зависании.
#[track_caller]
pub fn lock<T>(m: &Arc<Mutex<T>>) -> Tracked<'_, T> {
    let loc = Location::caller();
    let name = thread_name();
    with_state(|s| {
        s.waiting.insert(name.clone(), (loc, Instant::now()));
    });
    let guard = m.lock().unwrap_or_else(|e| e.into_inner());
    with_state(|s| {
        s.waiting.remove(&name);
        s.holder = Some((name, loc, Instant::now()));
    });
    Tracked { guard }
}

/// Отметка для отчёта: чем сейчас занят интерфейс.
pub fn ui_phase(phase: &'static str) {
    with_state(|s| {
        s.ui_phase = phase;
        s.ui_phase_since = Some(Instant::now());
    });
}

fn report_text() -> String {
    let now = Instant::now();
    let ms = |t: Instant| now.duration_since(t).as_millis();
    with_state(|s| {
        let mut out = format!("NovelTL {}: окно перестало отвечать.\n\n", env!("CARGO_PKG_VERSION"));
        match &s.holder {
            Some((th, loc, t)) => out += &format!("Замок контроллера держит поток «{th}» — {loc}, уже {} мс\n", ms(*t)),
            None => out += "Замок контроллера свободен\n",
        }
        if s.waiting.is_empty() {
            out += "Замок никто не ждёт\n";
        }
        for (th, (loc, t)) in &s.waiting {
            out += &format!("Ждёт замок поток «{th}» — {loc}, уже {} мс\n", ms(*t));
        }
        out += &format!(
            "Интерфейс: «{}» (с начала шага {} мс)\n",
            s.ui_phase,
            s.ui_phase_since.map(ms).unwrap_or(0)
        );
        out
    })
}

/// Фоновый сторож: раз в 2 с спрашивает главное окно; если оно не отвечает 5 с — пишет отчёт (один на зависание).
#[cfg(windows)]
pub fn start_watchdog(main_hwnd: impl Fn() -> isize + Send + 'static, reports_dir: std::path::PathBuf) {
    let _ = std::thread::Builder::new().name("watchdog".into()).spawn(move || {
        let mut reported = false;
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let hwnd = main_hwnd();
            if hwnd == 0 {
                continue;
            }
            let mut res = 0usize;
            let ok = unsafe {
                SendMessageTimeoutW(
                    HWND(hwnd as _),
                    WM_NULL,
                    WPARAM(0),
                    LPARAM(0),
                    SMTO_BLOCK | SMTO_ABORTIFHUNG,
                    5000,
                    Some(&mut res),
                )
            };
            if ok.0 != 0 {
                reported = false;
                continue;
            }
            if reported {
                continue;
            }
            reported = true;
            let _ = std::fs::create_dir_all(&reports_dir);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = std::fs::write(reports_dir.join(format!("hang_{stamp}.txt")), report_text());
        }
    });
}

/// В Linux сторожа нет (нет SendMessage): заглушка.
#[cfg(not(windows))]
pub fn start_watchdog(_main_hwnd: impl Fn() -> isize + Send + 'static, _reports_dir: std::path::PathBuf) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_shows_holder_and_phase() {
        let m = Arc::new(Mutex::new(5));
        ui_phase("проверка");
        let g = lock(&m);
        assert_eq!(*g, 5);
        let text = report_text();
        assert!(text.contains("Замок контроллера держит поток") && text.contains("diag.rs"), "{text}");
        assert!(text.contains("«проверка»"), "{text}");
        drop(g);
        assert!(report_text().contains("Замок контроллера свободен"));
    }
}
