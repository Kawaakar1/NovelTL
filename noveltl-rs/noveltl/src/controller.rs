//! Контроллер: связывает источник текста, автоподбор хука, перевод и плашку (перенос `noveltl/controller.py`).
//! Без Qt: интерфейс каждый кадр вызывает `poll()`, всё остальное происходит там.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use noveltl_core::cache::TranslationCache;
use noveltl_core::cleaner::clean;
use noveltl_core::net::{make_http, make_local_http, NET_SYSTEM};
use noveltl_core::pipeline::{LineResult, TranslationPipeline};
use noveltl_core::profiles::{
    exe_version, AppSettings, GlossaryEntry, OverlayFrac, Profile, ProfileStore,
};
use noveltl_core::langdetect::Lang;
use noveltl_core::scorer::{find_name_thread, HookKey, HookRef, ThreadTracker, NAME_PAIR_WINDOW};
use noveltl_core::translators::{Http, HttpError, HttpResponse, TranslatorManager};

use crate::hotkeys::{HotkeyEvent, Hotkeys};
use crate::overlay::{self, parse_color, Cmd, Frac, Overlay, Style};
use crate::paths::{data_dir, sub_data_dir};
use crate::source::{HookEvent, HookSource, Waker};
use crate::winutil::{self, WindowInfo};

const APP_NAME: &str = "NovelTL";
/// Пауза перед обработкой реплики: имя говорящего может прийти из соседнего потока чуть позже.
const LINE_DELAY: Duration = Duration::from_millis(120);
const SCAN_INTERVAL: Duration = Duration::from_millis(1500);

fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

// ---------- демо без интернета ----------

/// Подменяет сеть в демо: Google отвечает готовыми переводами сценария, DeepL — отказом.
pub struct DemoHttp {
    table_ru: HashMap<&'static str, &'static str>,
    table_en: HashMap<&'static str, &'static str>,
}

impl DemoHttp {
    pub fn new() -> Self {
        DemoHttp {
            table_ru: crate::demo::demo_translations_for(Lang::Ru),
            table_en: crate::demo::demo_translations_for(Lang::En),
        }
    }

    /// Демо-перевод по тексту (русский или английский), для неизвестного — заглушка.
    fn lookup(&self, q: &str, ru: bool) -> String {
        let table = if ru { &self.table_ru } else { &self.table_en };
        match table.get(q) {
            Some(t) => (*t).to_string(),
            None => format!("[демо] {q}"),
        }
    }
}

impl Default for DemoHttp {
    fn default() -> Self {
        Self::new()
    }
}

impl Http for DemoHttp {
    fn post_json(
        &self,
        url: &str,
        _headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<HttpResponse, HttpError> {
        // Microsoft: тело [{"Text": ...}], язык цели — в параметре to=
        if url.contains("api.cognitive.microsofttranslator.com") {
            let q = body.get(0).and_then(|t| t.get("Text")).and_then(|t| t.as_str()).unwrap_or("");
            let en = url.contains("to=en");
            let out = self.lookup(q, !en);
            let body = serde_json::json!([{ "translations": [{ "text": out, "to": if en { "en" } else { "ru" } }] }]);
            return Ok(HttpResponse { status: 200, body: body.to_string() });
        }
        // локальный ИИ: чат-запрос, текст реплики — в последнем сообщении пользователя
        if url.ends_with("/v1/chat/completions") {
            let msgs = body.get("messages").and_then(|m| m.as_array());
            let sys = msgs.and_then(|m| m.first()).and_then(|m| m.get("content")).and_then(|c| c.as_str()).unwrap_or("");
            let user = msgs.and_then(|m| m.last()).and_then(|m| m.get("content")).and_then(|c| c.as_str()).unwrap_or("");
            let q = user.rsplit_once("Translate this line:\n").map_or(user, |(_, t)| t);
            let out = self.lookup(q, !sys.contains("into English"));
            let body = serde_json::json!({"choices": [{"message": {"role": "assistant", "content": out}}]});
            return Ok(HttpResponse { status: 200, body: body.to_string() });
        }
        Ok(HttpResponse { status: 403, body: "{}".to_string() })
    }

    fn get(&self, _url: &str, query: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        let q = query.iter().find(|(k, _)| *k == "q").map(|(_, v)| *v).unwrap_or("");
        // язык ответа — по параметру tl: русский для "ru", иначе английский
        let tl = query.iter().find(|(k, _)| *k == "tl").map(|(_, v)| *v).unwrap_or("ru");
        let tr = self.lookup(q, tl == "ru");
        let body = serde_json::json!([[[tr, q, null]], null, "ja"]).to_string();
        Ok(HttpResponse { status: 200, body })
    }
}

// ---------- фоновый перевод ----------

pub enum WorkerCmd {
    SetPipeline(Option<TranslationPipeline>),
    SetDeeplKey(String),
    /// язык перевода
    SetTarget(Lang),
    /// режим выбора переводчика (см. `ENGINE_MODES`)
    SetMode(String),
    SetHttp(Arc<dyn Http>),
    /// клиент для локального ИИ (без прокси)
    SetLocalHttp(Arc<dyn Http>),
    /// ключ и регион Microsoft Translator
    SetMicrosoft(String, String),
    /// адрес и модель локального ИИ
    SetLocalAi(String, String),
    SetGlossary(Vec<GlossaryEntry>),
    Translate { seq: u64, name: String, text: String, use_cache: bool },
    Stop,
}

pub enum WorkerMsg {
    Done { seq: u64, result: LineResult },
    Failed { seq: u64, message: String },
}

struct Worker {
    tx: Sender<WorkerCmd>,
    rx: Receiver<WorkerMsg>,
    status: Arc<Mutex<String>>,
    applied: Arc<AtomicU64>,
    sent: u64,
    handle: Option<JoinHandle<()>>,
}

fn publish(status: &Mutex<String>, text: String) {
    *status.lock().unwrap_or_else(|e| e.into_inner()) = text;
}

fn worker_loop(
    rx: Receiver<WorkerCmd>,
    tx: Sender<WorkerMsg>,
    waker: Waker,
    status: Arc<Mutex<String>>,
    applied: Arc<AtomicU64>,
    deepl_key: String,
    target: Lang,
    mode: String,
    http: Arc<dyn Http>,
) {
    // «запасной» переводчик: нужен, чтобы показывать состояние, пока игры нет
    let mut idle = TranslatorManager::new(&deepl_key, None, http);
    idle.set_target(target);
    idle.set_mode(&mode);
    let mut pipeline: Option<TranslationPipeline> = None;
    publish(&status, idle.status_text());
    loop {
        let Ok(first) = rx.recv() else { return };
        let mut batch = vec![first];
        while let Ok(cmd) = rx.try_recv() {
            batch.push(cmd);
        }
        let count = batch.len() as u64;
        // «последний побеждает»: настройки применяются все по порядку, переводится только последняя реплика
        let mut job: Option<(u64, String, String, bool)> = None;
        let mut stop = false;
        for cmd in batch {
            match cmd {
                WorkerCmd::SetPipeline(p) => pipeline = p,
                WorkerCmd::SetDeeplKey(k) => {
                    idle.set_deepl_key(&k);
                    if let Some(p) = pipeline.as_mut() {
                        p.manager.set_deepl_key(&k);
                    }
                }
                WorkerCmd::SetTarget(t) => {
                    idle.set_target(t);
                    if let Some(p) = pipeline.as_mut() {
                        p.manager.set_target(t);
                    }
                }
                WorkerCmd::SetMode(m) => {
                    idle.set_mode(&m);
                    if let Some(p) = pipeline.as_mut() {
                        p.manager.set_mode(&m);
                    }
                }
                WorkerCmd::SetHttp(h) => {
                    idle.set_http(h.clone());
                    if let Some(p) = pipeline.as_mut() {
                        p.manager.set_http(h);
                    }
                }
                WorkerCmd::SetLocalHttp(h) => {
                    idle.set_local_http(h.clone());
                    if let Some(p) = pipeline.as_mut() {
                        p.manager.set_local_http(h);
                    }
                }
                WorkerCmd::SetMicrosoft(k, r) => {
                    idle.set_microsoft(&k, &r);
                    if let Some(p) = pipeline.as_mut() {
                        p.manager.set_microsoft(&k, &r);
                    }
                }
                WorkerCmd::SetLocalAi(u, m) => {
                    idle.set_local_ai(&u, &m);
                    if let Some(p) = pipeline.as_mut() {
                        p.manager.set_local_ai(&u, &m);
                    }
                }
                WorkerCmd::SetGlossary(g) => {
                    if let Some(p) = pipeline.as_mut() {
                        p.glossary = g;
                    }
                }
                WorkerCmd::Translate { seq, name, text, use_cache } => job = Some((seq, name, text, use_cache)),
                WorkerCmd::Stop => {
                    stop = true;
                    break;
                }
            }
        }
        if stop {
            return;
        }
        publish(&status, pipeline.as_ref().map_or_else(|| idle.status_text(), |p| p.manager.status_text()));
        if let Some((seq, name, text, use_cache)) = job {
            if let Some(p) = pipeline.as_mut() {
                let msg = match p.process(&name, &text, use_cache) {
                    Ok(result) => WorkerMsg::Done { seq, result },
                    Err(e) => {
                        let m = e.to_string();
                        WorkerMsg::Failed { seq, message: if m.is_empty() { "TranslateError".to_string() } else { m } }
                    }
                };
                let _ = tx.send(msg);
                waker();
                publish(&status, p.manager.status_text());
            }
        }
        applied.fetch_add(count, Ordering::SeqCst);
    }
}

impl Worker {
    fn start(waker: Waker, deepl_key: String, target: Lang, mode: String, http: Arc<dyn Http>) -> Worker {
        let (cmd_tx, cmd_rx) = channel();
        let (msg_tx, msg_rx) = channel();
        let status = Arc::new(Mutex::new(String::new()));
        let applied = Arc::new(AtomicU64::new(0));
        let (st, ap) = (status.clone(), applied.clone());
        let handle = std::thread::Builder::new()
            .name("translator".into())
            .spawn(move || worker_loop(cmd_rx, msg_tx, waker, st, ap, deepl_key, target, mode, http))
            .ok();
        Worker { tx: cmd_tx, rx: msg_rx, status, applied, sent: 0, handle }
    }

    fn send(&mut self, cmd: WorkerCmd) {
        if self.tx.send(cmd).is_ok() {
            self.sent += 1;
        }
    }

    /// Ждёт, пока поток переводчика разберёт все отправленные команды (но не дольше `timeout`).
    /// Возвращает `true`, если все команды разобраны вовремя.
    fn flush(&self, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        while self.applied.load(Ordering::SeqCst) < self.sent && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.applied.load(Ordering::SeqCst) >= self.sent
    }

    fn stop(&mut self) {
        let _ = self.tx.send(WorkerCmd::Stop);
        if let Some(h) = self.handle.take() {
            let until = Instant::now() + Duration::from_secs(3);
            while !h.is_finished() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            if h.is_finished() {
                let _ = h.join();
            }
        }
    }
}

// ---------- сессия ----------

pub struct Session {
    pub profile: Profile,
    pub pid: u32,
    pub hwnd: isize,
    pub title: String,
    /// профиль уже есть на диске
    pub saved: bool,
    /// идёт мастер настройки
    pub setup: bool,
    pub connected: bool,
    pub selected: Option<HookKey>,
    /// поток с именами говорящих
    pub name_key: Option<HookKey>,
    /// время последнего использованного имени
    pub name_used: f64,
    pub engine: String,
    pub warnings: Vec<String>,
    pub error: String,
    pub last_src: (String, String),
    pub last_result: Option<LineResult>,
    /// последние переведённые реплики, самая новая — в конце
    pub history: VecDeque<LineResult>,
    pub started: f64,
}

/// Сколько реплик хранит история диалога.
pub const HISTORY_LEN: usize = 50;

/// Добавляет реплику в историю; повторный перевод той же реплики заменяет запись.
fn push_history(history: &mut VecDeque<LineResult>, r: LineResult) {
    if history.back().is_some_and(|b| b.src_text == r.src_text && b.src_name == r.src_name) {
        history.pop_back();
    }
    history.push_back(r);
    while history.len() > HISTORY_LEN {
        history.pop_front();
    }
}

fn to_frac(f: OverlayFrac) -> Frac {
    Frac { x: f.x as f32, y: f.y as f32, w: f.w as f32, h: f.h as f32 }
}

fn from_frac(f: Frac) -> OverlayFrac {
    OverlayFrac { x: f.x as f64, y: f.y as f64, w: f.w as f64, h: f.h as f64 }
}

fn style_from(s: &AppSettings) -> Style {
    let d = Style::default();
    Style {
        font_family: s.font_family.clone(),
        font_size: s.font_size as f32,
        text_color: parse_color(&s.text_color, d.text_color),
        name_color: parse_color(&s.name_color, d.name_color),
        outline_color: parse_color(&s.outline_color, d.outline_color),
        bg_color: parse_color(&s.bg_color, d.bg_color),
        bg_opacity: s.bg_opacity.min(100) as u8,
    }
}

/// Сеть по настройкам; вместе с ней — сообщение пользователю, если что-то пришлось заменить.
fn build_network(s: &AppSettings) -> (Option<Arc<dyn Http>>, String) {
    let mut msg = String::new();
    let mut bind_ip = s.bind_ip.clone();
    if !bind_ip.is_empty() && !winutil::list_interfaces().iter().any(|(_, ip)| *ip == bind_ip) {
        bind_ip.clear();
        msg = "Сохранённый сетевой интерфейс сейчас недоступен — используется автоматический выбор".to_string();
    }
    let http: Option<Arc<dyn Http>> = match make_http(&s.net_mode, &s.proxy_url, &bind_ip, &s.proxy_user, &s.proxy_pass)
    {
        Ok(h) => Some(Arc::new(h)),
        Err(_) => {
            msg = "Настройки прокси неверные — используется подключение как в системе".to_string();
            make_http(NET_SYSTEM, "", "", "", "").ok().map(|h| Arc::new(h) as Arc<dyn Http>)
        }
    };
    (http, msg)
}

pub struct Controller {
    pub settings: AppSettings,
    pub store: ProfileStore,
    pub tracker: ThreadTracker,
    pub session: Option<Session>,
    pub hotkey_errors: Vec<String>,
    pub user_hidden: bool,
    /// появились новые данные о потоках (аналог threadsChanged)
    pub threads_dirty: bool,
    settings_path: PathBuf,
    source: Box<dyn HookSource>,
    hook_rx: Receiver<HookEvent>,
    overlay: Overlay,
    overlay_rx: Receiver<overlay::Event>,
    overlay_frac: OverlayFrac,
    hotkeys: Hotkeys,
    hotkey_rx: Receiver<HotkeyEvent>,
    worker: Worker,
    cache: Option<Arc<TranslationCache>>,
    http: Arc<dyn Http>,
    /// клиент локального ИИ (без прокси; в демо — DemoHttp)
    local_http: Arc<dyn Http>,
    demo: bool,
    demo_hwnd: isize,
    seq: u64,
    notices: Vec<String>,
    /// отложенные реплики: (когда обработать, сырой текст, время прихода)
    pending: Vec<(Instant, String, f64)>,
    last_scan: Instant,
    closed: bool,
    /// с какого момента окно игры потеряно, а новое не найдено
    window_lost_since: Option<Instant>,
    /// файлы кэша, которые не удалось удалить сразу
    stale_caches: Vec<PathBuf>,
}

impl Controller {
    pub fn new(
        source: Box<dyn HookSource>,
        hook_rx: Receiver<HookEvent>,
        demo: bool,
        demo_hwnd: isize,
        waker: Waker,
    ) -> Self {
        let settings_path = data_dir().join("settings.json");
        let settings = AppSettings::load(&settings_path);
        let store = ProfileStore::new(sub_data_dir("profiles"));
        let mut notices = Vec::new();
        let http: Arc<dyn Http> = if demo {
            Arc::new(DemoHttp::new())
        } else {
            let (h, msg) = build_network(&settings);
            if !msg.is_empty() {
                notices.push(msg);
            }
            match h {
                Some(h) => h,
                None => Arc::new(DemoHttp::new()), // сеть собрать не удалось совсем — переводить нечем
            }
        };
        // локальный ИИ ходит мимо прокси; в демо — тот же DemoHttp
        let local_http: Arc<dyn Http> = if demo { http.clone() } else { make_local_http() };
        let mut worker = Worker::start(
            waker.clone(),
            settings.deepl_key.clone(),
            settings.target_lang,
            settings.engine_mode.clone(),
            http.clone(),
        );
        worker.send(WorkerCmd::SetLocalHttp(local_http.clone()));
        worker.send(WorkerCmd::SetMicrosoft(settings.ms_key.clone(), settings.ms_region.clone()));
        worker.send(WorkerCmd::SetLocalAi(settings.local_ai_url.clone(), settings.local_ai_model.clone()));
        let (overlay, overlay_rx) = Overlay::start();
        overlay.send(Cmd::Style(style_from(&settings)));
        let (hk_tx, hotkey_rx) = channel();
        let hotkeys = Hotkeys::start(hk_tx, waker);
        let mut c = Controller {
            settings,
            store,
            tracker: ThreadTracker::default(),
            session: None,
            hotkey_errors: Vec::new(),
            user_hidden: false,
            threads_dirty: false,
            settings_path,
            source,
            hook_rx,
            overlay,
            overlay_rx,
            overlay_frac: OverlayFrac::default(),
            hotkeys,
            hotkey_rx,
            worker,
            cache: None,
            http,
            local_http,
            demo,
            demo_hwnd,
            seq: 0,
            notices,
            pending: Vec::new(),
            last_scan: Instant::now(),
            closed: false,
            window_lost_since: None,
            stale_caches: Vec::new(),
        };
        c.register_hotkeys();
        c
    }

    // ---------- опрос ----------

    /// Вызывается интерфейсом каждый кадр.
    pub fn poll(&mut self) {
        while let Ok(ev) = self.hook_rx.try_recv() {
            self.on_hook_event(ev);
        }
        while let Ok(msg) = self.worker.rx.try_recv() {
            match msg {
                WorkerMsg::Done { seq, result } => self.on_translated(seq, result),
                WorkerMsg::Failed { seq, message } => self.on_translate_failed(seq, message),
            }
        }
        while let Ok(ev) = self.overlay_rx.try_recv() {
            match ev {
                overlay::Event::Moved(f) => self.on_overlay_moved(f),
                overlay::Event::Failed(msg) => self.notice(format!("Не удалось создать плашку перевода: {msg}")),
            }
        }
        while let Ok(ev) = self.hotkey_rx.try_recv() {
            match ev {
                HotkeyEvent::Fired(0) => self.toggle_overlay(),
                HotkeyEvent::Fired(1) => self.retranslate(),
                HotkeyEvent::Fired(_) => {}
            }
        }
        self.run_pending();
        if self.last_scan.elapsed() >= SCAN_INTERVAL {
            self.last_scan = Instant::now();
            self.scan();
        }
    }

    /// HWND окна демо-«игры»: оно появляется уже после создания контроллера.
    pub fn set_demo_hwnd(&mut self, hwnd: isize) {
        self.demo_hwnd = hwnd;
    }

    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }

    pub fn translator_status(&self) -> String {
        self.worker.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn notice(&mut self, text: impl Into<String>) {
        self.notices.push(text.into());
    }

    // ---------- окна и сессии ----------

    pub fn windows(&self) -> Vec<WindowInfo> {
        let allowed: Vec<isize> = if self.demo_hwnd != 0 { vec![self.demo_hwnd] } else { Vec::new() };
        winutil::list_windows(&allowed)
    }

    fn scan(&mut self) {
        // повторяем удаление файлов кэша, которые были заняты
        self.stale_caches.retain(|p| match std::fs::remove_file(p) {
            Ok(()) => false,
            Err(e) => e.kind() != std::io::ErrorKind::NotFound,
        });
        if let Some(s) = &self.session {
            let (pid, hwnd) = (s.pid, s.hwnd);
            if !winutil::is_process_alive(pid) || (self.demo_hwnd != 0 && !winutil::is_window(hwnd)) {
                self.end_session();
            } else if !winutil::is_window(hwnd) {
                // окно пересоздано (например, смена режима экрана) — ищем новое
                let found = self.windows().into_iter().find(|w| w.pid == pid);
                if let Some(w) = found {
                    self.window_lost_since = None;
                    if let Some(s) = self.session.as_mut() {
                        s.hwnd = w.hwnd;
                    }
                    self.overlay.send(Cmd::Target(w.hwnd, None));
                } else {
                    // окна нет уже давно — завершаем сессию, дальше сработает автопоиск
                    let since = *self.window_lost_since.get_or_insert_with(Instant::now);
                    if since.elapsed() > Duration::from_secs(5) {
                        self.end_session();
                    }
                }
            } else {
                self.window_lost_since = None;
            }
            return;
        }
        // профили читаем с диска один раз за проход; у одной программы бывает несколько окон
        let profiles = self.store.all();
        if profiles.is_empty() {
            return;
        }
        let mut seen = std::collections::HashSet::new();
        for w in self.windows() {
            if !seen.insert(w.exe_path.to_lowercase()) {
                continue;
            }
            if let Some(profile) = self.store.find_in(&profiles, Path::new(&w.exe_path)) {
                if profile.hook.is_some() {
                    let name = profile.name.clone();
                    self.start_session(profile, &w, true, false);
                    self.notice(format!("Игра «{name}» найдена — перевод включён."));
                    break;
                }
            }
        }
    }

    fn make_pipeline(&self, lang: Lang, glossary: Vec<GlossaryEntry>) -> TranslationPipeline {
        let mut manager = TranslatorManager::new(&self.settings.deepl_key, self.cache.clone(), self.http.clone());
        manager.set_target(self.settings.target_lang);
        manager.set_mode(&self.settings.engine_mode);
        manager.set_microsoft(&self.settings.ms_key, &self.settings.ms_region);
        manager.set_local_ai(&self.settings.local_ai_url, &self.settings.local_ai_model);
        manager.set_local_http(self.local_http.clone());
        TranslationPipeline::new(manager, lang, glossary)
    }

    pub fn start_session(&mut self, profile: Profile, win: &WindowInfo, saved: bool, setup: bool) {
        self.end_session();
        self.window_lost_since = None;
        self.tracker.clear();
        let cache_path = sub_data_dir("cache").join(format!("{}.sqlite", profile.id));
        self.cache = TranslationCache::open(&cache_path).ok().map(Arc::new);
        let pipeline = self.make_pipeline(profile.lang, profile.glossary.clone());
        self.worker.send(WorkerCmd::SetPipeline(Some(pipeline)));
        let lang = profile.lang;
        self.overlay_frac = profile.overlay;
        let frac = to_frac(profile.overlay);
        self.session = Some(Session {
            profile,
            pid: win.pid,
            hwnd: win.hwnd,
            title: win.title.clone(),
            saved,
            setup,
            connected: false,
            selected: None,
            name_key: None,
            name_used: 0.0,
            engine: String::new(),
            warnings: Vec::new(),
            error: String::new(),
            last_src: (String::new(), String::new()),
            last_result: None,
            history: VecDeque::new(),
            started: now_secs(),
        });
        self.overlay.send(Cmd::Clear);
        self.overlay.send(Cmd::Target(win.hwnd, Some(frac)));
        self.source.attach(win.pid, lang.codepage());
    }

    pub fn end_session(&mut self) {
        // ответы воркера на реплики прошлой сессии больше не нужны
        self.seq += 1;
        self.pending.clear();
        self.window_lost_since = None;
        let Some(s) = self.session.take() else { return };
        self.worker.send(WorkerCmd::SetPipeline(None));
        self.cache = None;
        self.overlay.send(Cmd::EditMode(false));
        self.overlay.send(Cmd::Clear);
        self.overlay.send(Cmd::Target(0, None));
        self.source.detach(s.pid);
        self.tracker.clear();
    }

    // ---------- события источника ----------

    fn is_mine(&self, pid: u32) -> bool {
        self.session.as_ref().is_some_and(|s| s.pid == pid)
    }

    fn on_hook_event(&mut self, ev: HookEvent) {
        match ev {
            HookEvent::Connected(pid) => {
                if let Some(s) = self.session.as_mut().filter(|s| s.pid == pid) {
                    s.connected = true;
                    s.error.clear();
                }
            }
            HookEvent::Disconnected(pid) => {
                if self.is_mine(pid) {
                    self.end_session();
                }
            }
            HookEvent::AttachFailed(pid, message) => {
                if let Some(s) = self.session.as_mut().filter(|s| s.pid == pid) {
                    s.error = message.clone();
                    self.notice(message);
                }
            }
            HookEvent::HostInfo(kind, text) => {
                if let Some(s) = self.session.as_mut() {
                    if kind == crate::lunahost::HOSTINFO_ENGINE {
                        s.engine = text;
                    } else if kind == crate::lunahost::HOSTINFO_WARNING && s.warnings.len() < 50 {
                        s.warnings.push(text);
                    }
                }
            }
            HookEvent::ThreadCreated(info) => {
                if self.is_mine(info.pid) {
                    self.tracker.ensure(&info.code, &info.name, info.addr, info.ctx, info.ctx2);
                    self.threads_dirty = true;
                }
            }
            HookEvent::ThreadRemoved(_) => {}
            HookEvent::Text(info, text) => self.on_text(&info.code, &info.name, info.pid, info.addr, info.ctx, info.ctx2, text),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn on_text(&mut self, code: &str, name: &str, pid: u32, addr: u64, ctx: u64, ctx2: u64, text: String) {
        if !self.is_mine(pid) {
            return;
        }
        let key = self.tracker.add_text(code, name, addr, ctx, ctx2, &text, None);
        self.threads_dirty = true;
        let Some(s) = self.session.as_ref() else { return };
        if s.setup {
            return;
        }
        if s.selected.is_none() {
            self.select_hook();
        }
        let selected = self.session.as_ref().and_then(|s| s.selected.clone());
        if selected.as_ref() == Some(&key) {
            let t_line = self.tracker.get(&key).and_then(|st| st.lines.back().map(|l| l.0)).unwrap_or_else(now_secs);
            self.pending.push((Instant::now() + LINE_DELAY, text, t_line));
        }
    }

    fn select_hook(&mut self) {
        let Some(s) = self.session.as_mut() else { return };
        if let Some(key) = self.tracker.find_saved(s.profile.hook.as_ref()) {
            s.selected = Some(key);
            return;
        }
        let Some(pick) = self.tracker.auto_pick() else { return };
        let Some(st) = self.tracker.get(&pick.key) else { return };
        s.selected = Some(st.key());
        s.profile.set_hook(&st.hookcode, &st.hookname, st.ctx, st.ctx2);
        if s.saved {
            let _ = self.store.save(&s.profile);
        }
        self.notices.push("Сохранённый источник текста не нашёлся — NovelTL сам выбрал другой.".to_string());
    }

    fn run_pending(&mut self) {
        let now = Instant::now();
        let mut due = Vec::new();
        self.pending.retain(|p| {
            if p.0 <= now {
                due.push((p.1.clone(), p.2));
                false
            } else {
                true
            }
        });
        for (raw, t_line) in due {
            self.handle_line(&raw, true, false, Some(t_line));
        }
    }

    // ---------- перевод ----------

    /// Имя из отдельного потока, пришедшее вместе с этой репликой.
    fn companion_name(&mut self, t_line: f64) -> String {
        let Some(s) = self.session.as_mut() else { return String::new() };
        if s.name_key.is_none() {
            s.name_key = self.tracker.find_saved(s.profile.name_hook.as_ref());
        }
        if s.name_key.is_none() {
            let Some(selected) = s.selected.clone() else { return String::new() };
            let Some(key) = find_name_thread(&self.tracker, &selected) else { return String::new() };
            let Some(st) = self.tracker.get(&key) else { return String::new() };
            s.profile.name_hook = Some(HookRef { code: st.hookcode.clone(), name: st.hookname.clone(), ctx_lo: key.1, ctx2_lo: key.2 });
            s.name_key = Some(key);
            if s.saved {
                let _ = self.store.save(&s.profile);
            }
        }
        let Some(name_key) = s.name_key.clone() else { return String::new() };
        let Some(st) = self.tracker.get(&name_key) else { return String::new() };
        let Some((t_name, raw)) = st.lines.back() else { return String::new() };
        if *t_name <= s.name_used || (*t_name - t_line).abs() > NAME_PAIR_WINDOW {
            return String::new();
        }
        s.name_used = *t_name;
        let name = clean(raw).text;
        if name.chars().count() <= 30 {
            name
        } else {
            String::new()
        }
    }

    fn handle_line(&mut self, raw: &str, use_cache: bool, force: bool, t_line: Option<f64>) {
        let mut res = clean(raw);
        let Some(s) = self.session.as_ref() else { return };
        if s.setup || res.text.is_empty() {
            return;
        }
        if res.name.is_empty() && s.selected.is_some() {
            res.name = self.companion_name(t_line.unwrap_or_else(now_secs));
        }
        let src = (res.name.clone(), res.text.clone());
        let Some(s) = self.session.as_mut() else { return };
        if src == s.last_src && !force {
            return;
        }
        s.last_src = src;
        self.submit(res.name, res.text, use_cache);
    }

    fn submit(&mut self, name: String, text: String, use_cache: bool) {
        self.seq += 1;
        self.overlay.send(Cmd::Note { note: "…".to_string(), keep_text: true });
        let seq = self.seq;
        self.worker.send(WorkerCmd::Translate { seq, name, text, use_cache });
    }

    fn on_translated(&mut self, seq: u64, result: LineResult) {
        let Some(s) = self.session.as_mut() else { return };
        if seq != self.seq {
            return;
        }
        self.overlay.send(Cmd::Line { name: result.name.clone(), text: result.text.clone() });
        let notice = result.notice.clone();
        push_history(&mut s.history, result.clone());
        s.last_result = Some(result);
        if !notice.is_empty() {
            self.notices.push(notice);
        }
    }

    fn on_translate_failed(&mut self, seq: u64, message: String) {
        if self.session.is_some() && seq == self.seq {
            self.overlay.send(Cmd::Note { note: format!("⚠ Не удалось перевести: {message}"), keep_text: false });
        }
    }

    /// Перевести последнюю реплику заново, минуя кэш.
    pub fn retranslate(&mut self) {
        let Some(s) = self.session.as_ref() else { return };
        if !s.last_src.1.is_empty() {
            let (name, text) = s.last_src.clone();
            self.submit(name, text, false);
        }
    }

    pub fn toggle_overlay(&mut self) {
        self.user_hidden = !self.user_hidden;
        self.overlay.send(Cmd::UserHidden(self.user_hidden));
    }

    // ---------- мастер настройки ----------

    pub fn begin_setup(&mut self, win: &WindowInfo, lang: Lang, name: &str) {
        let exe = Path::new(&win.exe_path);
        let found = self.store.find_for_exe(exe);
        let saved = found.is_some();
        let mut profile = match found {
            Some(p) => p,
            None => {
                let version = exe_version(exe);
                let exe_name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                Profile {
                    id: self.store.new_id(&exe_name, &version),
                    name: if name.is_empty() { win.title.clone() } else { name.to_string() },
                    exe_name,
                    exe_path: win.exe_path.clone(),
                    version,
                    ..Profile::default()
                }
            }
        };
        profile.lang = lang;
        self.start_session(profile, win, saved, true);
    }

    /// Повторная настройка для уже подключённой игры.
    pub fn resume_setup(&mut self) -> bool {
        let Some(s) = self.session.as_mut() else { return false };
        s.setup = true;
        self.overlay.send(Cmd::Clear);
        true
    }

    pub fn choose_hook(&mut self, key: &HookKey) {
        let Some(s) = self.session.as_mut() else { return };
        let Some(st) = self.tracker.get(key) else { return };
        s.selected = Some(key.clone());
        s.profile.set_hook(&st.hookcode, &st.hookname, st.ctx, st.ctx2);
        // поток имён подберётся заново
        s.profile.name_hook = None;
        s.name_key = None;
    }

    pub fn finish_setup(&mut self) {
        let Some(s) = self.session.as_mut() else { return };
        s.profile.overlay = self.overlay_frac;
        let _ = self.store.save(&s.profile);
        s.saved = true;
        s.setup = false;
        let last = s
            .selected
            .as_ref()
            .and_then(|k| self.tracker.get(k))
            .and_then(|st| st.lines.back().map(|(t, raw)| (raw.clone(), *t)));
        self.overlay.send(Cmd::EditMode(false));
        if let Some((raw, t)) = last {
            self.handle_line(&raw, true, true, Some(t));
        }
    }

    pub fn cancel_setup(&mut self) {
        let Some(s) = self.session.as_ref() else { return };
        let (saved, id, hwnd) = (s.saved, s.profile.id.clone(), s.hwnd);
        self.overlay.send(Cmd::EditMode(false));
        if !saved {
            self.end_session();
            return;
        }
        if let Some(stored) = self.store.get(&id) {
            let pipeline = self.make_pipeline(stored.lang, stored.glossary.clone());
            self.worker.send(WorkerCmd::SetPipeline(Some(pipeline)));
            self.overlay_frac = stored.overlay;
            self.overlay.send(Cmd::Target(hwnd, Some(to_frac(stored.overlay))));
            if let Some(s) = self.session.as_mut() {
                s.profile = stored;
            }
        }
        if let Some(s) = self.session.as_mut() {
            s.setup = false;
            s.selected = self.tracker.find_saved(s.profile.hook.as_ref());
        }
    }

    pub fn set_overlay_edit(&mut self, on: bool) {
        self.overlay.send(Cmd::EditMode(on));
    }

    /// Вернуть плашку на место по умолчанию (запоминается при `finish_setup`).
    pub fn reset_overlay_position(&mut self) {
        self.overlay_frac = OverlayFrac::default();
        let hwnd = self.session.as_ref().map_or(0, |s| s.hwnd);
        self.overlay.send(Cmd::Target(hwnd, Some(to_frac(self.overlay_frac))));
    }

    /// Поставить плашку в заданное место (для снимков экрана).
    pub fn set_overlay_position(&mut self, frac: OverlayFrac) {
        self.overlay_frac = frac;
        let hwnd = self.session.as_ref().map_or(0, |s| s.hwnd);
        self.overlay.send(Cmd::Target(hwnd, Some(to_frac(frac))));
    }

    fn on_overlay_moved(&mut self, frac: Frac) {
        self.overlay_frac = from_frac(frac);
        if let Some(s) = self.session.as_mut() {
            if !s.setup && s.saved {
                s.profile.overlay = self.overlay_frac;
                let _ = self.store.save(&s.profile);
            }
        }
    }

    // ---------- профили и настройки ----------

    pub fn delete_profile(&mut self, id: &str) {
        let mut flushed = true;
        if self.session.as_ref().is_some_and(|s| s.profile.id == id) {
            self.end_session();
            // кэш открыт у переводчика — дожидаемся, пока он его отпустит
            flushed = self.worker.flush(Duration::from_secs(2));
        }
        self.store.delete(id);
        let path = sub_data_dir("cache").join(format!("{id}.sqlite"));
        let removed = match std::fs::remove_file(&path) {
            Ok(()) => true,
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
        };
        if !flushed || !removed {
            self.notice("Файл кэша этой игры занят — он будет удалён позже.");
            if !self.stale_caches.contains(&path) {
                self.stale_caches.push(path);
            }
        }
    }

    /// Меняет язык оригинала игры: в сессии и в хранилище. Для текущей сессии пересоздаётся pipeline;
    /// если сменилась кодовая страница — игра переподключается.
    pub fn set_profile_lang(&mut self, profile_id: &str, lang: Lang) {
        let in_session = self.session.as_ref().is_some_and(|s| s.profile.id == profile_id);
        if !in_session {
            // игра не запущена: достаточно поправить профиль на диске
            if let Some(mut p) = self.store.get(profile_id) {
                if p.lang != lang {
                    p.lang = lang;
                    let _ = self.store.save(&p);
                }
            }
            return;
        }
        let Some(s) = self.session.as_mut() else { return };
        if s.profile.lang == lang {
            return;
        }
        let codepage_changed = s.profile.lang.codepage() != lang.codepage();
        s.profile.lang = lang;
        let (saved, connected, has_hook) = (s.saved, s.connected, s.profile.hook.is_some());
        if saved {
            let _ = self.store.save(&s.profile);
        }
        let (profile, win_pid) = (s.profile.clone(), s.pid);
        if codepage_changed && connected {
            if saved && has_hook {
                // профиль уже обновлён на диске: завершаем сессию, автопоиск подключится заново с новой кодовой
                // страницей (пауза до следующего скана заодно пропускает запоздалый Disconnected от старой сессии)
                self.end_session();
                self.last_scan = Instant::now();
                self.notice("Язык игры изменён — NovelTL переподключается к игре.");
            } else {
                // профиля для автопоиска нет — подключаемся явно к тому же окну
                let win = self.windows().into_iter().find(|w| w.pid == win_pid);
                match win {
                    Some(win) => {
                        let setup = self.session.as_ref().is_some_and(|s| s.setup);
                        self.start_session(profile, &win, saved, setup);
                    }
                    None => self.end_session(),
                }
            }
            return;
        }
        // кодовая страница та же: хватает нового pipeline
        let pipeline = self.make_pipeline(lang, profile.glossary.clone());
        self.worker.send(WorkerCmd::SetPipeline(Some(pipeline)));
        self.retranslate();
    }

    pub fn save_glossary(&mut self, profile_id: &str, entries: Vec<GlossaryEntry>) {
        let in_session = self.session.as_ref().is_some_and(|s| s.profile.id == profile_id);
        let profile = if in_session {
            self.session.as_ref().map(|s| s.profile.clone())
        } else {
            self.store.get(profile_id)
        };
        let Some(mut profile) = profile else { return };
        profile.glossary = entries.clone();
        let _ = self.store.save(&profile);
        if in_session {
            if let Some(s) = self.session.as_mut() {
                s.profile.glossary = entries.clone();
            }
            self.worker.send(WorkerCmd::SetGlossary(entries));
            self.retranslate();
        }
    }

    /// Применяет сетевые настройки; возвращает сообщение, если что-то пришлось заменить.
    fn apply_network(&mut self) -> String {
        if self.demo {
            return String::new();
        }
        let (http, msg) = build_network(&self.settings);
        if let Some(h) = http {
            self.http = h.clone();
            self.worker.send(WorkerCmd::SetHttp(h));
        }
        msg
    }

    pub fn save_settings(&mut self, new: AppSettings) {
        let old = self.settings.clone();
        self.settings = new;
        self.register_hotkeys();
        // занятые другой программой сочетания откатываем на прежние, чтобы они продолжали работать
        let attempted = std::mem::take(&mut self.hotkey_errors);
        if !attempted.is_empty() {
            if attempted.contains(&self.settings.hotkey_toggle) {
                self.settings.hotkey_toggle = old.hotkey_toggle.clone();
            }
            if attempted.contains(&self.settings.hotkey_retranslate) {
                self.settings.hotkey_retranslate = old.hotkey_retranslate.clone();
            }
            self.register_hotkeys();
        }
        self.hotkey_errors = attempted;
        match self.settings.save_checked(&self.settings_path) {
            Ok(true) => self.notice(
                "Не удалось зашифровать ключ DeepL или пароль прокси — они сохранены открытым текстом в data\\settings.json.",
            ),
            Ok(false) => {}
            Err(e) => self.notice(format!("Не удалось сохранить настройки: {e}")),
        }
        self.worker.send(WorkerCmd::SetDeeplKey(self.settings.deepl_key.clone()));
        self.worker.send(WorkerCmd::SetTarget(self.settings.target_lang));
        self.worker.send(WorkerCmd::SetMode(self.settings.engine_mode.clone()));
        self.worker.send(WorkerCmd::SetMicrosoft(self.settings.ms_key.clone(), self.settings.ms_region.clone()));
        self.worker
            .send(WorkerCmd::SetLocalAi(self.settings.local_ai_url.clone(), self.settings.local_ai_model.clone()));
        let msg = self.apply_network();
        if !msg.is_empty() {
            self.notice(msg);
        }
        self.apply_overlay_style();
        // цель или режим сменились — плашка сразу показывает перевод по-новому
        let mode = self.settings.engine_mode.as_str();
        let ms_changed =
            mode == "microsoft" && (old.ms_key != self.settings.ms_key || old.ms_region != self.settings.ms_region);
        let ai_changed = mode == "local_ai"
            && (old.local_ai_url != self.settings.local_ai_url || old.local_ai_model != self.settings.local_ai_model);
        if self.session.is_some()
            && (old.target_lang != self.settings.target_lang
                || old.engine_mode != self.settings.engine_mode
                || ms_changed
                || ai_changed)
        {
            self.retranslate();
        }
    }

    /// Стиль плашки из `self.settings` (для предпросмотра без сохранения).
    pub fn apply_overlay_style(&mut self) {
        self.overlay.send(Cmd::Style(style_from(&self.settings)));
    }

    fn register_hotkeys(&mut self) {
        let combos = vec![self.settings.hotkey_toggle.clone(), self.settings.hotkey_retranslate.clone()];
        let failed = self.hotkeys.set(&combos);
        // пустое сочетание — «не задано», это не ошибка
        self.hotkey_errors = failed.into_iter().filter(|c| !c.is_empty()).collect();
    }

    // ---------- отчёт ----------

    pub fn build_report(&self) -> serde_json::Value {
        use serde_json::{json, Map, Value};
        let mut report = Map::new();
        report.insert("программа".into(), json!(format!("{APP_NAME} {}", env!("CARGO_PKG_VERSION"))));
        report.insert("время".into(), json!(local_time_text("%Y-%m-%d %H:%M:%S")));
        report.insert("переводчик".into(), json!(self.translator_status()));
        if let Some(s) = &self.session {
            let mut profile = serde_json::to_value(&s.profile).unwrap_or(Value::Null);
            if let Value::Object(m) = &mut profile {
                m.remove("exe_path"); // в пути может быть имя пользователя
                m.insert("glossary".into(), json!(s.profile.glossary.len()));
            }
            report.insert(
                "игра".into(),
                json!({
                    "окно": s.title,
                    "exe": s.profile.exe_name,
                    "разрядность": if winutil::is_process_64bit(s.pid) { 64 } else { 32 },
                    "движок": s.engine,
                    "подключено": s.connected,
                    "ошибка": s.error,
                    "предупреждения": s.warnings,
                    "секунд_с_подключения": (now_secs() - s.started).round() as i64,
                }),
            );
            report.insert("профиль".into(), profile);
            report.insert("выбранный_хук".into(), json!(s.selected));
            report.insert("хук_имён".into(), json!(s.name_key));
        }
        let ranked: HashMap<HookKey, _> = self.tracker.ranked().into_iter().map(|r| (r.key.clone(), r)).collect();
        let mut threads: Vec<(f64, Value)> = Vec::new();
        for st in self.tracker.iter() {
            let key = st.key();
            let r = ranked.get(&key);
            let score = r.map_or(0.0, |r| r.score);
            let mut details = Map::new();
            if let Some(r) = r {
                for (k, v) in &r.details {
                    details.insert(k.clone(), json!(v));
                }
            }
            threads.push((
                score,
                json!({
                    "код": st.hookcode,
                    "имя": st.hookname,
                    "addr": format!("0x{:x}", st.addr),
                    "ctx": format!("0x{:x}", st.ctx),
                    "ctx2": format!("0x{:x}", st.ctx2),
                    "ctx_lo": key.1,
                    "ctx2_lo": key.2,
                    "строк_всего": st.total,
                    "оценка": score,
                    "детали_оценки": Value::Object(details),
                    "последние_строки": st.last_raw(12),
                }),
            ));
        }
        threads.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        report.insert("потоки".into(), Value::Array(threads.into_iter().map(|t| t.1).collect()));
        Value::Object(report)
    }

    pub fn save_report(&self) -> std::io::Result<PathBuf> {
        let path = sub_data_dir("reports").join(format!("{}.json", local_time_text("report_%Y%m%d_%H%M%S")));
        let text = serde_json::to_string_pretty(&self.build_report()).map_err(std::io::Error::other)?;
        std::fs::write(&path, text)?;
        Ok(path)
    }

    // ---------- завершение ----------

    pub fn shutdown(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let _ = self.hotkeys.set(&[]);
        self.hotkeys.stop();
        self.end_session();
        self.source.shutdown();
        self.worker.stop();
        self.overlay.send(Cmd::Quit);
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Местное время по шаблону с %Y %m %d %H %M %S.
fn local_time_text(fmt: &str) -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    fmt.replace("%Y", &format!("{:04}", t.wYear))
        .replace("%m", &format!("{:02}", t.wMonth))
        .replace("%d", &format!("{:02}", t.wDay))
        .replace("%H", &format!("{:02}", t.wHour))
        .replace("%M", &format!("{:02}", t.wMinute))
        .replace("%S", &format!("{:02}", t.wSecond))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::{demo_translations, DemoSource, DEMO_TITLE, SCRIPT};
    use crate::paths::set_data_dir_override;

    #[test]
    fn history_keeps_last_lines_and_replaces_retranslation() {
        let line = |src: &str, text: &str| LineResult {
            name: String::new(),
            text: text.into(),
            src_name: String::new(),
            src_text: src.into(),
            engine: "google",
            cached: false,
            notice: String::new(),
        };
        let mut h = VecDeque::new();
        for i in 0..HISTORY_LEN + 5 {
            push_history(&mut h, line(&i.to_string(), "x"));
        }
        assert_eq!(h.len(), HISTORY_LEN);
        assert_eq!(h.front().unwrap().src_text, "5");
        // повторный перевод последней реплики не дублирует её
        push_history(&mut h, line(&(HISTORY_LEN + 4).to_string(), "y"));
        assert_eq!(h.len(), HISTORY_LEN);
        assert_eq!(h.back().unwrap().text, "y");
    }

    // проверка на этапе компиляции: Controller можно передавать между потоками
    fn _assert_send() {
        fn is_send<T: Send>() {}
        is_send::<Controller>();
    }

    /// Крутит `poll()`, пока не выполнится условие или не кончится время.
    fn pump(c: &mut Controller, ms: u64, cond: impl Fn(&Controller) -> bool) -> bool {
        let until = Instant::now() + Duration::from_millis(ms);
        loop {
            c.poll();
            if cond(c) {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn demo_http_answers() {
        let h = DemoHttp::new();
        let r = h.get("x", &[("q", "灯里"), ("sl", "ja")]).unwrap();
        assert_eq!(r.status, 200);
        let v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
        assert_eq!(v[0][0][0], "Акари");
        let r = h.get("x", &[("q", "неизвестно")]).unwrap();
        assert!(r.body.contains("[демо] неизвестно"));
        assert_eq!(h.post_json("x", &[], &serde_json::json!({})).unwrap().status, 403);
        // цель en — английские варианты
        let r = h.get("x", &[("q", "灯里"), ("sl", "ja"), ("tl", "en")]).unwrap();
        let v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
        assert_eq!(v[0][0][0], "Akari");
    }

    #[test]
    fn controller_demo_flow() {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("noveltl_ctl_{}_{}", std::process::id(), nanos));
        let _ = std::fs::create_dir_all(&dir);
        set_data_dir_override(dir.clone());

        let waker: Waker = Arc::new(|| {});
        let (tx, rx) = channel();
        let src = DemoSource::new(tx, waker.clone());
        let mut c = Controller::new(Box::new(src.clone()), rx, true, 0, waker);

        let exe = std::env::current_exe().unwrap();
        let win = WindowInfo {
            hwnd: 0,
            pid: std::process::id(),
            title: DEMO_TITLE.to_string(),
            exe_path: exe.to_string_lossy().into_owned(),
        };

        // 1. мастер: подключение
        c.begin_setup(&win, Lang::Ja, "Демо");
        assert!(pump(&mut c, 3000, |c| c.session.as_ref().is_some_and(|s| s.connected)), "нет подключения");

        // 2. реплики и автоподбор
        for i in 0..=3 {
            src.emit_line(i);
            pump(&mut c, 300, |_| false);
        }
        let pick = c.tracker.auto_pick().expect("автоподбор не выбрал хук");
        assert_eq!(pick.key.0, "HQ-1C*0@0:kirikiri.exe");
        assert_eq!(pick.key.2, 1);

        // 3. выбор и сохранение
        c.choose_hook(&pick.key);
        c.finish_setup();
        let id = c.session.as_ref().unwrap().profile.id.clone();
        let stored = c.store.get(&id).expect("профиль не сохранён");
        assert_eq!(stored.hook.as_ref().map(|h| h.code.as_str()), Some("HQ-1C*0@0:kirikiri.exe"));
        assert_eq!(stored.hook.as_ref().map(|h| h.key()), Some(pick.key.clone()));
        assert!(c.session.as_ref().is_some_and(|s| s.saved && !s.setup));

        // 4. перевод пятой реплики
        src.emit_line(4);
        let want = demo_translations()[SCRIPT[4].text];
        let ok = pump(&mut c, 3000, |c| {
            c.session.as_ref().and_then(|s| s.last_result.as_ref()).is_some_and(|r| r.text == want)
        });
        assert!(ok, "нет перевода: {:?}", c.session.as_ref().and_then(|s| s.last_result.clone()));
        let report = c.build_report();
        assert!(report["потоки"].as_array().is_some_and(|a| a.len() >= 3));
        assert!(report["игра"]["подключено"].as_bool() == Some(true));
        assert!(report["профиль"].get("exe_path").is_none());
        assert!(c.translator_status().starts_with("Google"));

        // 4а. смена цели на английский: плашка переводится заново на английский (SetTarget + retranslate)
        let mut st = c.settings.clone();
        st.target_lang = Lang::En;
        c.save_settings(st);
        let want_en = crate::demo::demo_translations_for(Lang::En)[SCRIPT[4].text];
        let ok = pump(&mut c, 3000, |c| {
            c.session.as_ref().and_then(|s| s.last_result.as_ref()).is_some_and(|r| r.text == want_en)
        });
        assert!(ok, "нет английского перевода: {:?}", c.session.as_ref().and_then(|s| s.last_result.clone()));
        // возвращаем русский для дальнейших шагов
        let mut st = c.settings.clone();
        st.target_lang = Lang::Ru;
        c.save_settings(st);
        let want_ru = demo_translations()[SCRIPT[4].text];
        pump(&mut c, 3000, |c| {
            c.session.as_ref().and_then(|s| s.last_result.as_ref()).is_some_and(|r| r.text == want_ru)
        });

        // 4б. локальный ИИ (модель-заглушка) в демо: перевод на английский приходит от него, а не от Google
        let mut st = c.settings.clone();
        st.engine_mode = "local_ai".to_string();
        st.local_ai_model = "qwen2.5:7b".to_string();
        st.target_lang = Lang::En;
        c.save_settings(st);
        let ok = pump(&mut c, 3000, |c| {
            c.session
                .as_ref()
                .and_then(|s| s.last_result.as_ref())
                .is_some_and(|r| r.text == want_en && r.engine == "local_ai")
        });
        assert!(ok, "нет перевода локальным ИИ: {:?}", c.session.as_ref().and_then(|s| s.last_result.clone()));

        // 4в. Microsoft (ключ-заглушка): русский перевод
        let mut st = c.settings.clone();
        st.engine_mode = "microsoft".to_string();
        st.ms_key = "test-key".to_string();
        st.target_lang = Lang::Ru;
        c.save_settings(st);
        let ok = pump(&mut c, 3000, |c| {
            c.session
                .as_ref()
                .and_then(|s| s.last_result.as_ref())
                .is_some_and(|r| r.text == want_ru && r.engine == "microsoft")
        });
        assert!(ok, "нет перевода Microsoft: {:?}", c.session.as_ref().and_then(|s| s.last_result.clone()));
        let mut st = c.settings.clone();
        st.engine_mode = "auto".to_string();
        c.save_settings(st);

        // 5. перезапуск сессии с сохранённым профилем
        c.end_session();
        pump(&mut c, 200, |_| false); // забрать Disconnected от прошлой сессии
        c.start_session(stored, &win, true, false);
        assert!(pump(&mut c, 3000, |c| c.session.as_ref().is_some_and(|s| s.connected)), "нет повторного подключения");
        src.emit_line(5);
        assert!(pump(&mut c, 3000, |c| c.session.as_ref().is_some_and(|s| s.selected.is_some())));
        assert_eq!(c.session.as_ref().unwrap().selected, Some(pick.key.clone()));
        assert!(!c.session.as_ref().unwrap().setup);

        // ответ со старым seq (из прошлой сессии) не попадает в новую
        let stale = LineResult {
            name: String::new(),
            text: "чужой".to_string(),
            src_name: String::new(),
            src_text: String::new(),
            engine: "test",
            cached: false,
            notice: String::new(),
        };
        let old_seq = c.seq - 1;
        c.session.as_mut().unwrap().last_result = None;
        c.on_translated(old_seq, stale);
        assert!(c.session.as_ref().unwrap().last_result.is_none(), "старый ответ протёк в новую сессию");

        // отчёт на диск
        let path = c.save_report().expect("отчёт не записан");
        assert!(path.exists());

        // 6.
        c.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
