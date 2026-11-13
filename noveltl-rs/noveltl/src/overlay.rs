//! Плашка с переводом поверх окна игры.
//!
//! Родное окно Windows с попиксельной прозрачностью (UpdateLayeredWindow), текст — DirectWrite
//! через Direct2D (системный подбор шрифтов, японский, контур). Своё окно и свой поток: плашка
//! не забирает фокус, клики проходят сквозь неё в игру. В режиме настройки и при открытом замочке
//! (значок в углу плашки) её можно двигать и растягивать мышью средствами самой Windows (WM_NCHITTEST).
//! Рядом с замочком — кнопка меню: те же пункты, что у значка в трее.

use noveltl_core::i18n::t;
use std::cell::RefCell;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{BOOL, COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN,
    D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, ID2D1RenderTarget, ID2D1StrokeStyle,
    D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_DASH, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_LINE_JOIN_ROUND,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL,
    D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_DASH_STYLE_SOLID,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextLayout, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DIB_RGB_COLORS, HDC,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

pub use crate::overlay_common::*;

/// Управление плашкой из любого потока.
#[derive(Clone)]
pub struct Overlay {
    tx: Sender<Cmd>,
    hwnd: Arc<AtomicIsize>,
}

impl Overlay {
    pub fn start() -> (Overlay, Receiver<Event>) {
        let (tx, rx) = channel::<Cmd>();
        let (etx, erx) = channel::<Event>();
        let hwnd = Arc::new(AtomicIsize::new(0));
        let hwnd2 = hwnd.clone();
        std::thread::Builder::new()
            .name("overlay".into())
            .spawn(move || run(rx, etx, hwnd2))
            .expect("поток плашки");
        (Overlay { tx, hwnd }, erx)
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
        let h = self.hwnd.load(Ordering::Acquire);
        if h != 0 {
            unsafe {
                let _ = PostMessageW(HWND(h as _), WM_APP, WPARAM(0), LPARAM(0));
            }
        }
    }

    pub fn hwnd(&self) -> isize {
        self.hwnd.load(Ordering::Acquire)
    }
}

// ---------------- поток плашки ----------------

const EDGE: i32 = 10;
const MIN_W: i32 = 220;
const MIN_H: i32 = 70;
const PAD: f32 = 18.0;
const MIN_FONT: f32 = 11.0;
/// кнопки в правом верхнем углу (замочек, меню): размер, отступ от краёв и между кнопками
const LOCK: i32 = 22;
const LOCK_INSET: i32 = 12;
const BTN_GAP: i32 = 10;
const BTN_LOCK: usize = 0;
const BTN_MENU: usize = 1;
const BUTTONS: usize = 2;
/// пункты меню плашки: (команда, id пункта меню трея, не используется); 0 — разделитель
const MENU: [(u32, &str, &str); 6] = [
    (1, crate::tray::ID_OPEN, ""),
    (0, "", ""),
    (2, crate::tray::ID_TOGGLE, ""),
    (3, crate::tray::ID_RETR, ""),
    (4, crate::tray::ID_SETTINGS, ""),
    (5, crate::tray::ID_QUIT, ""),
];


struct State {
    hwnd: HWND,
    rx: Receiver<Cmd>,
    events: Sender<Event>,
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    dashes: ID2D1StrokeStyle,
    /// сплошная линия с круглыми концами (дужка замка, полоски меню)
    round: ID2D1StrokeStyle,
    target: isize,
    frac: Frac,
    style: Style,
    name: String,
    text: String,
    /// второй перевод (мелко под основным)
    alt: String,
    note: String,
    edit: bool,
    /// замочек открыт: плашку можно двигать во время игры
    unlocked: bool,
    /// курсор над плашкой / над кнопкой (замочек или меню)
    hover: bool,
    hover_btn: Option<usize>,
    user_hidden: bool,
    force_active: bool,
    visible: bool,
    in_size_move: bool,
    game: RECT,
    drawn: Option<(i32, i32, u64)>, // размер и «отпечаток» содержимого последней отрисовки
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn run(rx: Receiver<Cmd>, events: Sender<Event>, out: Arc<AtomicIsize>) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        // ранний выход: сообщаем контроллеру и освобождаем COM
        macro_rules! bail {
            ($msg:expr) => {{
                let _ = events.send(Event::Failed($msg));
                windows::Win32::System::Com::CoUninitialize();
                return;
            }};
        }
        let module = match GetModuleHandleW(None) {
            Ok(m) => m,
            Err(e) => bail!(format!("GetModuleHandleW: {e}")),
        };
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            hCursor: LoadCursorW(None, IDC_SIZEALL).unwrap_or_default(),
            lpszClassName: w!("NovelTLOverlay"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
        let hwnd = match CreateWindowExW(
            ex,
            w!("NovelTLOverlay"),
            w!("NovelTL — плашка"),
            WS_POPUP,
            0, 0, 400, 120,
            None, None, module, None,
        ) {
            Ok(h) => h,
            Err(e) => bail!(format!("CreateWindowExW: {e}")),
        };

        let d2d: ID2D1Factory = match D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) {
            Ok(f) => f,
            Err(e) => bail!(format!("Direct2D: {e}")),
        };
        let dw: IDWriteFactory = match DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) {
            Ok(f) => f,
            Err(e) => bail!(format!("DirectWrite: {e}")),
        };
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let rt = match d2d.CreateDCRenderTarget(&props) {
            Ok(r) => r,
            Err(e) => bail!(format!("CreateDCRenderTarget: {e}")),
        };
        let dashes = match d2d.CreateStrokeStyle(
            &D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_DASH,
                dashOffset: 0.0,
            },
            None,
        ) {
            Ok(d) => d,
            Err(e) => bail!(format!("CreateStrokeStyle: {e}")),
        };
        let round = match d2d.CreateStrokeStyle(
            &D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
            },
            None,
        ) {
            Ok(d) => d,
            Err(e) => bail!(format!("CreateStrokeStyle: {e}")),
        };

        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                hwnd,
                rx,
                events,
                d2d,
                dw,
                rt,
                dashes,
                round,
                target: 0,
                frac: Frac::default(),
                style: Style::default(),
                name: String::new(),
                text: String::new(),
                alt: String::new(),
                note: String::new(),
                edit: false,
                unlocked: false,
                hover: false,
                hover_btn: None,
                user_hidden: false,
                force_active: false,
                visible: false,
                in_size_move: false,
                game: RECT::default(),
                drawn: None,
            })
        });
        out.store(hwnd.0 as isize, Ordering::Release);
        SetTimer(hwnd, 1, 50, None);
        // команды, отправленные до появления окна, ждут в канале — забираем их сразу
        let _ = PostMessageW(hwnd, WM_APP, WPARAM(0), LPARAM(0));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        STATE.with(|s| *s.borrow_mut() = None);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_APP => {
            if with_state(|st| st.drain()) == Some(true) {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if with_state(|st| {
                let quit = st.drain();
                st.sync();
                quit
            }) == Some(true)
            {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_NCHITTEST => {
            let x = (lp.0 & 0xFFFF) as i16 as i32;
            let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut r = RECT::default();
            let _ = GetWindowRect(hwnd, &mut r);
            let (movable, lock_shown) = with_state(|st| (st.movable(), st.lock_shown())).unwrap_or((false, false));
            if lock_shown && button_at(r.right - r.left, x - r.left, y - r.top).is_some() {
                return LRESULT(HTCLIENT as isize);
            }
            if !movable {
                return LRESULT(HTTRANSPARENT as isize);
            }
            let (l, t, rr, b) = (x - r.left <= EDGE, y - r.top <= EDGE, r.right - x <= EDGE, r.bottom - y <= EDGE);
            let code = match (l, t, rr, b) {
                (true, true, _, _) => HTTOPLEFT,
                (_, true, true, _) => HTTOPRIGHT,
                (true, _, _, true) => HTBOTTOMLEFT,
                (_, _, true, true) => HTBOTTOMRIGHT,
                (true, _, _, _) => HTLEFT,
                (_, _, true, _) => HTRIGHT,
                (_, true, _, _) => HTTOP,
                (_, _, _, true) => HTBOTTOM,
                _ => HTCAPTION,
            };
            LRESULT(code as isize)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_SETCURSOR if (lp.0 & 0xFFFF) as u32 == HTCLIENT => {
            // над кнопками — «рука»
            SetCursor(LoadCursorW(None, IDC_HAND).unwrap_or_default());
            LRESULT(1)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32);
            let mut r = RECT::default();
            let _ = GetClientRect(hwnd, &mut r);
            let shown = with_state(|st| st.lock_shown()).unwrap_or(false);
            match button_at(r.right, x, y).filter(|_| shown) {
                Some(BTN_LOCK) => {
                    with_state(|st| st.toggle_lock());
                }
                Some(BTN_MENU) => {
                    // меню показываем вне заимствования состояния: у него свой цикл сообщений
                    if let Some((target, events)) = with_state(|st| (st.target, st.events.clone())) {
                        let mut p = POINT { x: r.right - LOCK_INSET - LOCK, y: LOCK_INSET + LOCK + 4 };
                        let _ = ClientToScreen(hwnd, &mut p);
                        if let Some(id) = show_menu(hwnd, target, p) {
                            let _ = events.send(Event::Menu(id));
                        }
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_ENTERSIZEMOVE => {
            with_state(|st| st.in_size_move = true);
            LRESULT(0)
        }
        WM_SIZING => {
            // не даём сжать меньше минимума
            let r = &mut *(lp.0 as *mut RECT);
            let edge = wp.0 as u32;
            if r.right - r.left < MIN_W {
                if matches!(edge, WMSZ_LEFT | WMSZ_TOPLEFT | WMSZ_BOTTOMLEFT) { r.left = r.right - MIN_W } else { r.right = r.left + MIN_W }
            }
            if r.bottom - r.top < MIN_H {
                if matches!(edge, WMSZ_TOP | WMSZ_TOPLEFT | WMSZ_TOPRIGHT) { r.top = r.bottom - MIN_H } else { r.bottom = r.top + MIN_H }
            }
            LRESULT(1)
        }
        WM_WINDOWPOSCHANGED => {
            with_state(|st| {
                if st.visible {
                    st.render(false);
                }
            });
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_EXITSIZEMOVE => {
            with_state(|st| {
                st.in_size_move = false;
                st.report_geometry();
            });
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn hwnd_of(v: isize) -> HWND {
    HWND(v as _)
}

impl State {
    /// Применяет команды. true — пора закрываться.
    fn drain(&mut self) -> bool {
        let mut redraw = false;
        while let Ok(cmd) = self.rx.try_recv() {
            match cmd {
                Cmd::Target(h, frac) => {
                    self.target = h;
                    if let Some(f) = frac {
                        self.frac = f;
                    }
                }
                Cmd::Line { name, text, alt } => {
                    self.name = name;
                    self.text = text;
                    self.alt = alt;
                    self.note.clear();
                }
                Cmd::Note { note, keep_text } => {
                    self.note = note;
                    if !keep_text {
                        self.name.clear();
                        self.text.clear();
                        self.alt.clear();
                    }
                }
                Cmd::Clear => {
                    self.name.clear();
                    self.text.clear();
                    self.alt.clear();
                    self.note.clear();
                }
                Cmd::EditMode(on) => {
                    self.edit = on;
                    self.update_click_through();
                }
                Cmd::UserHidden(h) => self.user_hidden = h,
                Cmd::Style(s) => self.style = s,
                Cmd::ForceActive(on) => self.force_active = on,
                Cmd::Quit => return true,
            }
            redraw = true;
        }
        if redraw {
            self.drawn = None;
            self.sync();
        }
        false
    }

    /// Можно двигать и растягивать мышью.
    fn movable(&self) -> bool {
        self.edit || self.unlocked
    }

    /// Замочек рисуем во время игры; в мастере плашка и так двигается.
    fn lock_shown(&self) -> bool {
        !self.edit
    }

    fn toggle_lock(&mut self) {
        self.unlocked = !self.unlocked;
        self.update_click_through();
        self.render(false);
    }

    /// Клики проходят сквозь плашку, кроме режима перемещения и наведения на кнопки.
    fn update_click_through(&self) {
        self.set_click_through(!(self.movable() || (self.lock_shown() && self.hover_btn.is_some())));
    }

    /// Следит за курсором: подсветка замочка и кликабельность только над ним.
    fn track_mouse(&mut self) {
        let mut p = POINT::default();
        let mut r = RECT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
            let _ = GetWindowRect(self.hwnd, &mut r);
        }
        let hover = self.visible && p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom;
        let hover_btn = if hover { button_at(r.right - r.left, p.x - r.left, p.y - r.top) } else { None };
        if (hover, hover_btn) != (self.hover, self.hover_btn) {
            self.hover = hover;
            self.hover_btn = hover_btn;
            self.update_click_through();
        }
    }

    fn set_click_through(&self, on: bool) {
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let t = WS_EX_TRANSPARENT.0 as isize;
            let new = if on { ex | t } else { ex & !t };
            if new != ex {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, new);
            }
        }
    }

    fn game_client_rect(&self) -> Option<RECT> {
        unsafe {
            if self.target == crate::clipboard::CLIPBOARD_HWND {
                // у буфера обмена нет окна игры: место плашки считается от рабочей области экрана
                let mut r = RECT::default();
                SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut r as *mut RECT as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0))
                    .ok()?;
                return (r.right > r.left && r.bottom > r.top).then_some(r);
            }
            let h = hwnd_of(self.target);
            let mut r = RECT::default();
            GetClientRect(h, &mut r).ok()?;
            if r.right <= 0 || r.bottom <= 0 {
                return None;
            }
            let mut p = POINT { x: 0, y: 0 };
            if !ClientToScreen(h, &mut p).as_bool() {
                return None;
            }
            Some(RECT { left: p.x, top: p.y, right: p.x + r.right, bottom: p.y + r.bottom })
        }
    }

    fn should_show(&self) -> bool {
        unsafe {
            if self.target == 0 {
                return false;
            }
            let clipboard = self.target == crate::clipboard::CLIPBOARD_HWND;
            let h = hwnd_of(self.target);
            if !clipboard && (!IsWindow(h).as_bool() || IsIconic(h).as_bool()) {
                return false;
            }
            let has_content = self.edit || !self.text.is_empty() || !self.note.is_empty();
            if !has_content || (self.user_hidden && !self.edit) {
                return false;
            }
            if clipboard {
                // окна игры нет — плашка видна всегда, когда есть текст
                return true;
            }
            if self.force_active || self.edit {
                // в режиме настройки окно NovelTL (мастер) обычно активно — плашку всё равно показываем
                return true;
            }
            // пока плашку тянут мышью, активной может стать она сама
            let fg = GetForegroundWindow();
            fg == h || fg == self.hwnd
        }
    }

    /// Следит за окном игры: положение, видимость.
    fn sync(&mut self) {
        if self.in_size_move {
            return;
        }
        if !self.should_show() {
            if self.visible {
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
                self.visible = false;
            }
            return;
        }
        let Some(game) = self.game_client_rect() else { return };
        self.game = game;
        self.track_mouse();
        let gw = (game.right - game.left) as f32;
        let gh = (game.bottom - game.top) as f32;
        let f = self.frac;
        let x = game.left + (f.x * gw).round() as i32;
        let y = game.top + (f.y * gh).round() as i32;
        let w = ((f.w * gw).round() as i32).max(MIN_W);
        let h = ((f.h * gh).round() as i32).max(MIN_H);
        unsafe {
            let mut cur = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut cur);
            if cur.left != x || cur.top != y || cur.right - cur.left != w || cur.bottom - cur.top != h || !self.visible {
                let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
                self.visible = true;
            }
        }
        self.render(false);
    }

    fn report_geometry(&mut self) {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut r);
        }
        let g = self.game;
        let gw = (g.right - g.left) as f32;
        let gh = (g.bottom - g.top) as f32;
        if gw <= 0.0 || gh <= 0.0 {
            return;
        }
        let round = |v: f32| (v * 10000.0).round() / 10000.0;
        let frac = Frac {
            x: round(((r.left - g.left) as f32 / gw).clamp(-0.5, 1.0)),
            y: round(((r.top - g.top) as f32 / gh).clamp(-0.5, 1.0)),
            w: round(((r.right - r.left) as f32 / gw).clamp(0.05, 1.5)),
            h: round(((r.bottom - r.top) as f32 / gh).clamp(0.05, 1.5)),
        };
        self.frac = frac;
        let _ = self.events.send(Event::Moved(frac));
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&self.name, &self.text, &self.alt, &self.note, self.edit, &self.style.font_family).hash(&mut h);
        (self.unlocked, self.hover, self.hover_btn).hash(&mut h);
        (self.style.font_size as u32, self.style.text_color, self.style.name_color).hash(&mut h);
        (self.style.outline_color, self.style.bg_color, self.style.bg_opacity).hash(&mut h);
        h.finish()
    }

    /// Рисует содержимое в памяти и выводит его в окно с попиксельной прозрачностью.
    fn render(&mut self, force: bool) {
        let mut wr = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut wr);
        }
        let (w, h) = (wr.right - wr.left, wr.bottom - wr.top);
        if w <= 0 || h <= 0 {
            return;
        }
        let fp = self.fingerprint();
        if !force && self.drawn == Some((w, h, fp)) {
            return;
        }
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(screen);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let Ok(dib) = CreateDIBSection(mem, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(mem);
                ReleaseDC(None, screen);
                return;
            };
            let old = SelectObject(mem, dib);
            if self.draw(mem, w, h).is_ok() {
                let pos = POINT { x: wr.left, y: wr.top };
                let size = SIZE { cx: w, cy: h };
                let src = POINT { x: 0, y: 0 };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let _ = UpdateLayeredWindow(
                    self.hwnd, screen, Some(&pos), Some(&size), mem, Some(&src),
                    COLORREF(0), Some(&blend), ULW_ALPHA,
                );
                self.drawn = Some((w, h, fp));
            }
            SelectObject(mem, old);
            let _ = DeleteObject(dib);
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
        }
    }

    fn draw(&self, dc: HDC, w: i32, h: i32) -> windows::core::Result<()> {
        unsafe {
            self.rt.BindDC(dc, &RECT { left: 0, top: 0, right: w, bottom: h })?;
            let rt: &ID2D1RenderTarget = &self.rt;
            rt.BeginDraw();
            rt.Clear(Some(&color(0, 0.0)));
            let s = &self.style;
            let full = D2D_RECT_F { left: 1.0, top: 1.0, right: w as f32 - 1.0, bottom: h as f32 - 1.0 };
            let bg = rt.CreateSolidColorBrush(&color(s.bg_color, s.bg_opacity.min(100) as f32 / 100.0), None)?;
            rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: full, radiusX: 12.0, radiusY: 12.0 }, &bg);

            let (name, text) = if self.edit && self.text.is_empty() {
                (t("Акари"), t("«Так будет выглядеть перевод. Перетащите плашку на окно диалога игры и растяните по размеру»."))
            } else {
                (self.name.as_str(), self.text.as_str())
            };
            // справа место под кнопки, чтобы они не закрывали конец первой строки
            let right_pad = if self.lock_shown() { (LOCK_INSET + BUTTONS as i32 * (LOCK + BTN_GAP)) as f32 } else { PAD };
            let inner_w = (w as f32 - PAD - right_pad).max(10.0);
            let inner_h = (h as f32 - 1.2 * PAD).max(10.0);
            let (left, top) = (PAD, 0.6 * PAD);

            if !text.is_empty() {
                // автоуменьшение шрифта, пока текст не влезет
                let mut size = s.font_size.max(MIN_FONT);
                loop {
                    let name_size = (size * 0.8).max(MIN_FONT);
                    let name_layout = if name.is_empty() { None } else { Some(self.layout(name, name_size, true, inner_w, inner_h)?) };
                    let body = self.layout(text, size, false, inner_w, inner_h)?;
                    // второй перевод — мельче и бледнее, под основным (в режиме настройки — нет)
                    let alt_size = (size * 0.7).max(MIN_FONT);
                    let alt = if self.alt.is_empty() || self.edit { None } else { Some(self.layout(&self.alt, alt_size, false, inner_w, inner_h)?) };
                    let name_h = match &name_layout { Some(l) => metrics(l)?.height + 2.0, None => 0.0 };
                    let body_h = metrics(&body)?.height;
                    let alt_h = match &alt { Some(l) => metrics(l)?.height + 4.0, None => 0.0 };
                    if name_h + body_h + alt_h <= inner_h || size <= MIN_FONT {
                        let mut y = top;
                        if let Some(l) = &name_layout {
                            self.outlined(rt, l, left, y, s.name_color, name_size)?;
                            y += name_h;
                        }
                        self.outlined(rt, &body, left, y, s.text_color, size)?;
                        if let Some(l) = &alt {
                            self.outlined(rt, l, left, y + body_h + 4.0, 0xC9CBE0, alt_size)?;
                        }
                        break;
                    }
                    size -= 1.0;
                }
            }
            if !self.note.is_empty() {
                let small = if text.is_empty() { 18.0 } else { 15.0 };
                let l = self.layout(&self.note, small, false, inner_w, inner_h)?;
                let m = metrics(&l)?;
                let (x, y) = if text.is_empty() {
                    (left, top + 4.0)
                } else {
                    (left + inner_w - m.width, h as f32 - m.height - 6.0)
                };
                self.outlined(rt, &l, x, y, 0xC9CBE0, small)?;
            }
            if self.lock_shown() {
                self.draw_buttons(rt, w)?;
            }
            if self.movable() {
                // рамка режима перемещения — белая, в тон чёрно-белому интерфейсу
                let accent = rt.CreateSolidColorBrush(&color(0xFFFFFF, 0.9), None)?;
                rt.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: full, radiusX: 12.0, radiusY: 12.0 }, &accent, 2.0, &self.dashes);
                let (l, t, r, b) = (full.left + 3.0, full.top + 3.0, full.right - 3.0, full.bottom - 3.0);
                let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
                for (x, y) in [(l, t), (cx, t), (r, t), (l, cy), (r, cy), (l, b), (cx, b), (r, b)] {
                    rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x, y }, radiusX: 5.0, radiusY: 5.0 }, &accent);
                }
            }
            rt.EndDraw(None, None)
        }
    }

    /// Кнопки в углу: замочек (закрытый — плашка закреплена, открытый — её можно двигать
    /// и растягивать) и меню.
    fn draw_buttons(&self, rt: &ID2D1RenderTarget, w: i32) -> windows::core::Result<()> {
        unsafe {
            for i in 0..BUTTONS {
                let hot = self.hover_btn == Some(i);
                // заметны, когда мышь над плашкой или замок открыт; иначе почти не мешают тексту
                let alpha = if hot { 1.0 } else if self.hover || self.unlocked { 0.85 } else { 0.3 };
                let r = button_rect(w, i);
                let (l, t, s) = (r.left as f32, r.top as f32, LOCK as f32);
                if hot {
                    let back = rt.CreateSolidColorBrush(&color(0x000000, 0.45), None)?;
                    let rect = D2D_RECT_F { left: l - 4.0, top: t - 4.0, right: l + s + 4.0, bottom: t + s + 4.0 };
                    rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect, radiusX: 6.0, radiusY: 6.0 }, &back);
                }
                let white = rt.CreateSolidColorBrush(&color(0xFFFFFF, alpha), None)?;
                if i == BTN_MENU {
                    // три полоски
                    for k in 0..3 {
                        let y = t + 6.0 + k as f32 * 5.0;
                        rt.DrawLine(D2D_POINT_2F { x: l + 4.0, y }, D2D_POINT_2F { x: l + s - 4.0, y }, &white, 2.4, &self.round);
                    }
                    continue;
                }
                // дужка — перевёрнутая «U»; у открытого замка приподнята, правая нога висит над корпусом
                let (lx, rx, rad) = (l + 6.5, l + s - 6.5, (s - 13.0) / 2.0);
                let lift = if self.unlocked { 4.0 } else { 0.0 };
                let top_y = t + 2.5 + rad - lift; // центр дуги по вертикали
                let body_top = t + 10.0;
                let right_end = if self.unlocked { body_top - 3.5 } else { body_top + 1.0 };
                let geo = self.d2d.CreatePathGeometry()?;
                let sink = geo.Open()?;
                sink.BeginFigure(D2D_POINT_2F { x: lx, y: body_top + 1.0 }, D2D1_FIGURE_BEGIN_HOLLOW);
                sink.AddLine(D2D_POINT_2F { x: lx, y: top_y });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F { x: rx, y: top_y },
                    size: D2D_SIZE_F { width: rad, height: rad },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
                sink.AddLine(D2D_POINT_2F { x: rx, y: right_end });
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
                sink.Close()?;
                rt.DrawGeometry(&geo, &white, 2.6, &self.round);
                // корпус с замочной скважиной
                let body = D2D_RECT_F { left: l + 2.5, top: body_top, right: l + s - 2.5, bottom: t + s };
                rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: body, radiusX: 3.0, radiusY: 3.0 }, &white);
                let dark = rt.CreateSolidColorBrush(&color(0x000000, alpha), None)?;
                let hole = D2D_POINT_2F { x: l + s / 2.0, y: body_top + (t + s - body_top) / 2.0 };
                rt.FillEllipse(&D2D1_ELLIPSE { point: hole, radiusX: 2.0, radiusY: 2.0 }, &dark);
            }
        }
        Ok(())
    }

    fn layout(&self, text: &str, size: f32, bold: bool, w: f32, h: f32) -> windows::core::Result<IDWriteTextLayout> {
        unsafe {
            let weight = if bold { DWRITE_FONT_WEIGHT_BOLD } else { DWRITE_FONT_WEIGHT_NORMAL };
            let format = self.dw.CreateTextFormat(
                &HSTRING::from(self.style.font_family.as_str()),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("ru-ru"),
            )?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            let wide: Vec<u16> = text.encode_utf16().collect();
            self.dw.CreateTextLayout(&wide, &format, w, h * 4.0)
        }
    }

    /// Текст с контуром: контур — тот же текст, нарисованный со сдвигами по кругу.
    fn outlined(&self, rt: &ID2D1RenderTarget, layout: &IDWriteTextLayout, x: f32, y: f32, rgb: u32, size: f32) -> windows::core::Result<()> {
        unsafe {
            let outline = rt.CreateSolidColorBrush(&color(self.style.outline_color, 1.0), None)?;
            let fill = rt.CreateSolidColorBrush(&color(rgb, 1.0), None)?;
            let r = (size / 13.0).max(1.2);
            for ring in [r, r * 0.55] {
                for i in 0..16 {
                    let a = i as f32 / 16.0 * std::f32::consts::TAU;
                    rt.DrawTextLayout(D2D_POINT_2F { x: x + a.cos() * ring, y: y + a.sin() * ring }, layout, &outline, D2D1_DRAW_TEXT_OPTIONS_NONE);
                }
            }
            rt.DrawTextLayout(D2D_POINT_2F { x, y }, layout, &fill, D2D1_DRAW_TEXT_OPTIONS_NONE);
            Ok(())
        }
    }
}

/// Где кнопка `i` (0 — замочек, 1 — меню) внутри плашки шириной `w` (координаты плашки).
fn button_rect(w: i32, i: usize) -> RECT {
    let right = w - LOCK_INSET - i as i32 * (LOCK + BTN_GAP);
    RECT { left: right - LOCK, top: LOCK_INSET, right, bottom: LOCK_INSET + LOCK }
}

/// Какая кнопка в точке (x, y) плашки шириной `w`; с запасом в 4 px, чтобы легко попасть.
fn button_at(w: i32, x: i32, y: i32) -> Option<usize> {
    (0..BUTTONS).find(|&i| {
        let b = button_rect(w, i);
        x >= b.left - 4 && x < b.right + 4 && y >= b.top - 4 && y < b.bottom + 4
    })
}

/// Меню плашки у точки `p` (экран). Возвращает id выбранного пункта меню трея.
fn show_menu(hwnd: HWND, game: isize, p: POINT) -> Option<&'static str> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        for (cmd, id, _) in MENU {
            if cmd == 0 {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            } else {
                let _ = AppendMenuW(menu, MF_STRING, cmd as usize, &HSTRING::from(menu_label(id)));
            }
        }
        // без этого меню не закрывается кликом мимо него (особенность TrackPopupMenu)
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTALIGN | TPM_TOPALIGN | TPM_NONOTIFY, p.x, p.y, 0, hwnd, None);
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        let id = MENU.iter().find(|(c, _, _)| *c != 0 && *c as i32 == cmd.0).map(|(_, id, _)| *id);
        // фокус — обратно игре, если пользователь не идёт в окно NovelTL
        if !matches!(id, Some(crate::tray::ID_OPEN | crate::tray::ID_SETTINGS)) && game != 0 {
            let _ = SetForegroundWindow(hwnd_of(game));
        }
        id
    }
}

fn metrics(l: &IDWriteTextLayout) -> windows::core::Result<DWRITE_TEXT_METRICS> {
    let mut m = DWRITE_TEXT_METRICS::default();
    unsafe { l.GetMetrics(&mut m)? };
    Ok(m)
}

fn color(rgb: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((rgb >> 16) & 0xFF) as f32 / 255.0,
        g: ((rgb >> 8) & 0xFF) as f32 / 255.0,
        b: (rgb & 0xFF) as f32 / 255.0,
        a,
    }
}

#[allow(dead_code)]
fn _unused(_: PCWSTR, _: BOOL) {}
