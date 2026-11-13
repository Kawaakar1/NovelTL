//! Источник текста: обёртка над LunaHost64.dll (движок LunaHook, GPL-3.0). Перенос `noveltl/lunahost.py`.

use noveltl_core::i18n::{t, tf};
use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, c_void};
use std::panic::catch_unwind;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use windows::core::{s, PCWSTR};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW, SetDllDirectoryW};

use crate::paths::files_dir;
use crate::source::{HookEvent, HookSource, ThreadInfo, Waker};
use crate::winutil;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const HOSTINFO_WARNING: i32 = 1;
pub const HOSTINFO_ENGINE: i32 = 4;

pub fn defender_hint() -> &'static str {
    t("Игра не ответила за 10 секунд. Чаще всего внедрение блокирует Защитник Windows: \
добавьте папку NovelTL и папку игры в исключения \
(Безопасность Windows → Защита от вирусов и угроз → Управление настройками → Исключения), \
затем перезапустите игру и NovelTL.")
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ThreadParam {
    pub process_id: u32,
    pub addr: u64,
    pub ctx: u64,
    pub ctx2: u64,
}

type ProcessEvent = extern "C" fn(u32);
type ThreadEventMaybeEmbed = extern "C" fn(*const u16, *const c_char, ThreadParam, bool);
type ThreadEvent = extern "C" fn(*const u16, *const c_char, ThreadParam);
type OutputCallback = extern "C" fn(*const u16, *const c_char, ThreadParam, *const u16);
type HostInfoHandler = extern "C" fn(i32, *const u16);
type HookInsertHandler = extern "C" fn(u32, u64, *const u16);
type EmbedCallback = extern "C" fn(*const u16, ThreadParam);
type I18NQueryCallback = extern "C" fn(*const u16) -> *mut c_void;
type EmuGameInfoCallback = extern "C" fn(*const u16, *const u16, *const u16);

type LunaStart = unsafe extern "C" fn(
    ProcessEvent,
    ProcessEvent,
    ThreadEventMaybeEmbed,
    ThreadEvent,
    OutputCallback,
    HostInfoHandler,
    HookInsertHandler,
    EmbedCallback,
    I18NQueryCallback,
    EmuGameInfoCallback,
);
type LunaSettings = unsafe extern "C" fn(i32, i32, i32, i32, bool);
type LunaConnectProcess = unsafe extern "C" fn(u32);
type LunaCheckIfNeedInject = unsafe extern "C" fn(u32) -> bool;
type LunaDetachProcess = unsafe extern "C" fn(u32);

#[derive(Clone, Copy)]
struct Api {
    settings: LunaSettings,
    connect_process: LunaConnectProcess,
    check_if_need_inject: LunaCheckIfNeedInject,
    detach_process: LunaDetachProcess,
}

// ---- глобальное состояние для колбэков (они приходят из чужих потоков) ----

type ConnSignal = Arc<(Mutex<bool>, Condvar)>;

struct Shared {
    tx: Sender<HookEvent>,
    waker: Waker,
    connected: HashMap<u32, ConnSignal>,
    attached: HashSet<u32>,
    /// поколение подключения для каждого pid: поздние сигналы старого воркера игнорируются
    gens: HashMap<u32, u64>,
    next_gen: u64,
}

static SHARED: Mutex<Option<Shared>> = Mutex::new(None);

fn lock_shared() -> std::sync::MutexGuard<'static, Option<Shared>> {
    SHARED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Отправить событие и разбудить интерфейс.
fn emit(ev: HookEvent) {
    let (tx, waker) = {
        let g = lock_shared();
        match g.as_ref() {
            Some(s) => (s.tx.clone(), s.waker.clone()),
            None => return,
        }
    };
    let _ = tx.send(ev);
    waker();
}

unsafe fn wide_to_string(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    // ограничение длины, чтобы не бежать по памяти без конца
    const MAX_LEN: usize = 1_048_576;
    let mut n = 0usize;
    while n < MAX_LEN && *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

unsafe fn thread_info(code: *const u16, name: *const c_char, tp: ThreadParam) -> ThreadInfo {
    let name = if name.is_null() {
        String::new()
    } else {
        // ограничение длины, как в wide_to_string: не бежим по памяти без конца
        const MAX_NAME: usize = 4096;
        let mut n = 0usize;
        while n < MAX_NAME && *name.add(n) != 0 {
            n += 1;
        }
        String::from_utf8_lossy(std::slice::from_raw_parts(name.cast::<u8>(), n)).into_owned()
    };
    ThreadInfo { code: wide_to_string(code), name, pid: tp.process_id, addr: tp.addr, ctx: tp.ctx, ctx2: tp.ctx2 }
}

extern "C" fn on_connect(pid: u32) {
    let _ = catch_unwind(|| {
        let sig = lock_shared().as_ref().and_then(|s| s.connected.get(&pid).cloned());
        if let Some(sig) = sig {
            *sig.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
            sig.1.notify_all();
        }
        emit(HookEvent::Connected(pid));
    });
}

extern "C" fn on_disconnect(pid: u32) {
    let _ = catch_unwind(|| {
        if let Some(s) = lock_shared().as_mut() {
            s.attached.remove(&pid);
            s.connected.remove(&pid);
            s.gens.remove(&pid);
        }
        emit(HookEvent::Disconnected(pid));
    });
}

extern "C" fn on_create(code: *const u16, name: *const c_char, tp: ThreadParam, _embedable: bool) {
    let _ = catch_unwind(|| {
        let info = unsafe { thread_info(code, name, tp) };
        emit(HookEvent::ThreadCreated(info));
    });
}

extern "C" fn on_destroy(code: *const u16, name: *const c_char, tp: ThreadParam) {
    let _ = catch_unwind(|| {
        let info = unsafe { thread_info(code, name, tp) };
        emit(HookEvent::ThreadRemoved(info));
    });
}

extern "C" fn on_output(code: *const u16, name: *const c_char, tp: ThreadParam, text: *const u16) {
    let _ = catch_unwind(|| {
        let (info, text) = unsafe { (thread_info(code, name, tp), wide_to_string(text)) };
        emit(HookEvent::Text(info, text));
    });
}

extern "C" fn on_hostinfo(kind: i32, text: *const u16) {
    let _ = catch_unwind(|| {
        let text = unsafe { wide_to_string(text) };
        emit(HookEvent::HostInfo(kind, text));
    });
}

extern "C" fn on_hook_insert(_pid: u32, _addr: u64, _code: *const u16) {}
extern "C" fn on_embed(_text: *const u16, _tp: ThreadParam) {}
extern "C" fn on_i18n(_text: *const u16) -> *mut c_void {
    std::ptr::null_mut()
}
extern "C" fn on_emu_game(_a: *const u16, _b: *const u16, _c: *const u16) {}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct LunaHost {
    api: Option<Api>,
    error: String,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl LunaHost {
    pub fn start(events: Sender<HookEvent>, waker: Waker) -> LunaHost {
        *lock_shared() = Some(Shared {
            tx: events,
            waker,
            connected: HashMap::new(),
            attached: HashSet::new(),
            gens: HashMap::new(),
            next_gen: 0,
        });
        match Self::load() {
            Ok(api) => LunaHost { api: Some(api), error: String::new(), workers: Vec::new() },
            Err(e) => LunaHost { api: None, error: e, workers: Vec::new() },
        }
    }

    fn load() -> Result<Api, String> {
        let dir = files_dir();
        let path = dir.join("LunaHost64.dll");
        if !path.is_file() {
            return Err(tf("Не найден файл {}", &[&path.display()]));
        }
        unsafe {
            // чтобы рядом лежащие msvcp140/vcruntime подтянулись
            let wdir = to_wide(&dir.to_string_lossy());
            let _ = SetDllDirectoryW(PCWSTR(wdir.as_ptr()));
            let wpath = to_wide(&path.to_string_lossy());
            let module = LoadLibraryW(PCWSTR(wpath.as_ptr()))
                .map_err(|e| tf("Не удалось загрузить LunaHost64.dll: {}", &[&e]))?;

            macro_rules! func {
                ($name:expr, $ty:ty) => {
                    match GetProcAddress(module, $name) {
                        Some(f) => std::mem::transmute::<unsafe extern "system" fn() -> isize, $ty>(f),
                        None => return Err(t("Не удалось загрузить LunaHost64.dll: нет нужной функции").to_string()),
                    }
                };
            }
            let start: LunaStart = func!(s!("Luna_Start"), LunaStart);
            let api = Api {
                settings: func!(s!("Luna_Settings"), LunaSettings),
                connect_process: func!(s!("Luna_ConnectProcess"), LunaConnectProcess),
                check_if_need_inject: func!(s!("Luna_CheckIfNeedInject"), LunaCheckIfNeedInject),
                detach_process: func!(s!("Luna_DetachProcess"), LunaDetachProcess),
            };
            start(
                on_connect,
                on_disconnect,
                on_create,
                on_destroy,
                on_output,
                on_hostinfo,
                on_hook_insert,
                on_embed,
                on_i18n,
                on_emu_game,
            );
            Ok(api)
        }
    }
}

/// Сбой подключения: действует, только если поколение ещё актуально.
fn fail_attach(pid: u32, my_gen: u64, text: String) {
    {
        let mut g = lock_shared();
        let Some(s) = g.as_mut() else { return };
        if s.gens.get(&pid) != Some(&my_gen) {
            return;
        }
        s.attached.remove(&pid);
    }
    emit(HookEvent::AttachFailed(pid, text));
}

fn attach_worker(api: Api, pid: u32, codepage: u32, sig: ConnSignal, my_gen: u64) {
    let fail = |text: String| fail_attach(pid, my_gen, text);
    unsafe {
        (api.settings)(500, codepage as i32, 3000, 1_000_000, false);
        (api.connect_process)(pid);
        if (api.check_if_need_inject)(pid) {
            let bits = if winutil::is_process_64bit(pid) { "64" } else { "32" };
            let dir = files_dir();
            let exe = dir.join(format!("LunaSubprocess{bits}.exe")).to_string_lossy().into_owned();
            let hook_dll = dir.join(format!("LunaHook{bits}.dll")).to_string_lossy().into_owned();
            let pid_s = pid.to_string();
            let mut code = winutil::run_hidden(&exe, &["dllinject", &pid_s, &hook_dll], 30);
            if code != 1 {
                // обычного доступа не хватило — пробуем с правами администратора
                code = winutil::run_elevated(&exe, &format!("dllinject {pid} \"{hook_dll}\""), 30_000);
            }
            if code != 1 {
                fail(tf(
                    "Не удалось подключиться к игре (код {}). Если игра запущена от имени \
администратора — запустите так же NovelTL. {}",
                    &[&code, &defender_hint()],
                ));
                return;
            }
        }
    }
    let (lock, cv) = &*sig;
    let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let (guard, _) = cv
        .wait_timeout_while(guard, CONNECT_TIMEOUT, |ok| !*ok)
        .unwrap_or_else(|e| e.into_inner());
    if !*guard {
        drop(guard);
        fail(defender_hint().to_string());
    }
}

impl HookSource for LunaHost {
    fn attach(&mut self, pid: u32, codepage: u32) {
        let Some(api) = self.api else {
            let text = if self.error.is_empty() { t("Движок LunaHook не загружен.").to_string() } else { self.error.clone() };
            emit(HookEvent::AttachFailed(pid, text));
            return;
        };
        let sig: ConnSignal = Arc::new((Mutex::new(false), Condvar::new()));
        let my_gen = {
            let mut g = lock_shared();
            let Some(s) = g.as_mut() else { return };
            if s.attached.contains(&pid) {
                drop(g);
                emit(HookEvent::Connected(pid));
                return;
            }
            s.attached.insert(pid);
            s.connected.insert(pid, sig.clone());
            s.next_gen += 1;
            s.gens.insert(pid, s.next_gen);
            s.next_gen
        };
        // убираем уже завершённые потоки
        self.workers.retain(|h| !h.is_finished());
        let handle = std::thread::spawn(move || {
            let r = catch_unwind(std::panic::AssertUnwindSafe(|| attach_worker(api, pid, codepage, sig, my_gen)));
            if r.is_err() {
                fail_attach(pid, my_gen, t("Ошибка подключения: внутренний сбой").to_string());
            }
        });
        self.workers.push(handle);
    }

    fn detach(&mut self, pid: u32) {
        let Some(api) = self.api else { return };
        let was = lock_shared()
            .as_mut()
            .map(|s| {
                // поколение и сигнал убираем: поздние сигналы старого воркера станут пустыми
                s.gens.remove(&pid);
                s.connected.remove(&pid);
                s.attached.remove(&pid)
            })
            .unwrap_or(false);
        if was {
            unsafe { (api.detach_process)(pid) };
        }
    }

    fn shutdown(&mut self) {
        let pids: Vec<u32> = lock_shared().as_ref().map(|s| s.attached.iter().copied().collect()).unwrap_or_default();
        for pid in pids {
            self.detach(pid);
        }
        // ждём потоки подключения не дольше 3 секунд суммарно, незавершённые бросаем
        let until = std::time::Instant::now() + Duration::from_secs(3);
        while self.workers.iter().any(|h| !h.is_finished()) && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        for h in std::mem::take(&mut self.workers) {
            if h.is_finished() {
                let _ = h.join();
            }
        }
    }

    fn error(&self) -> String {
        self.error.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Instant;

    /// Python с tkinter для живого теста: переменная NOVELTL_TEST_PYTHON или `python` из PATH.
    fn python() -> String {
        std::env::var("NOVELTL_TEST_PYTHON").unwrap_or_else(|_| "python".into())
    }
    const TEST_WINDOW: &str = r#"import tkinter as tk, ctypes
r = tk.Tk(); r.title('NovelTL inject test'); r.geometry('400x200')
lines = ['おはようございます。', '今日はいい天気ですね。', 'そろそろ行きましょうか。', 'はい、分かりました。']
u = ctypes.windll.user32; u.GetDC.restype = ctypes.c_void_p
def tick(i=0):
    s = lines[i % len(lines)]
    hdc = u.GetDC(ctypes.c_void_p(r.winfo_id()))
    ctypes.windll.gdi32.TextOutW(ctypes.c_void_p(hdc), 10, 120, s, len(s)); r.after(1200, tick, i + 1)
r.after(1500, tick); r.mainloop()
"#;

    #[test]
    #[ignore]
    fn live_inject() {
        let mut child = std::process::Command::new(python())
            .args(["-I", "-c", TEST_WINDOW])
            .spawn()
            .expect("не запустился python");
        let pid = child.id();
        std::thread::sleep(Duration::from_secs(2));

        let (tx, rx) = channel();
        let mut host = LunaHost::start(tx, Arc::new(|| {}));
        assert_eq!(host.error(), "", "ошибка загрузки");
        host.attach(pid, 932);

        let mut events = Vec::new();
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if let Ok(ev) = rx.recv_timeout(Duration::from_millis(100)) {
                events.push(ev);
            }
        }
        host.shutdown();
        let _ = child.kill();
        let _ = child.wait();

        let connected = events.iter().any(|e| matches!(e, HookEvent::Connected(p) if *p == pid));
        let threads = events.iter().filter(|e| matches!(e, HookEvent::ThreadCreated(_))).count();
        let texts: Vec<&String> = events
            .iter()
            .filter_map(|e| if let HookEvent::Text(_, t) = e { Some(t) } else { None })
            .collect();
        println!("Connected: {connected}, потоков: {threads}, текстов: {}", texts.len());
        for t in texts.iter().take(4) {
            println!("  текст: {t}");
        }
        for e in &events {
            if let HookEvent::AttachFailed(_, m) = e {
                println!("AttachFailed: {m}");
            }
        }
        assert!(connected, "нет Connected");
        assert!(threads > 0, "нет ThreadCreated");
        assert!(!texts.is_empty(), "нет Text");
        let jp = |t: &&String| t.chars().any(|c| ('\u{3040}'..='\u{30ff}').contains(&c) || ('\u{4e00}'..='\u{9fff}').contains(&c));
        assert!(texts.iter().any(jp), "нет японского текста");
    }
}
