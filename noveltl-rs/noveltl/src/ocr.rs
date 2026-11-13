//! OCR как запасной путь: чтение текста диалога с экрана встроенным распознаванием Windows.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use noveltl_core::i18n::{t, tf};
use noveltl_core::langdetect::Lang;
use windows::core::HSTRING;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT,
    DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use crate::source::{HookEvent, ThreadInfo, Waker};

pub const OCR_CODE: &str = "OCR";

/// «Поток» текста OCR для трекера.
pub fn thread_info(pid: u32) -> ThreadInfo {
    ThreadInfo {
        code: OCR_CODE.to_string(),
        name: t("Текст с экрана (OCR)").to_string(),
        pid,
        ..Default::default()
    }
}

fn create_engine(lang: Lang) -> Result<OcrEngine, String> {
    let tag = match lang {
        Lang::Ja | Lang::Auto => "ja",
        Lang::Zh => "zh-Hans",
        Lang::Ko => "ko",
        Lang::En => "en-US",
        Lang::Ru => "ru",
    };
    let missing = || tf("В Windows не установлено распознавание текста для языка «{}». Откройте «Параметры» → «Время и язык» → «Язык и регион», добавьте этот язык и включите для него «Оптическое распознавание символов».", &[&lang.title()]);
    let language = Language::CreateLanguage(&HSTRING::from(tag)).map_err(|e| e.to_string())?;
    if !OcrEngine::IsLanguageSupported(&language).unwrap_or(false) {
        return Err(missing());
    }
    OcrEngine::TryCreateFromLanguage(&language).map_err(|_| missing())
}

/// Есть ли в Windows распознавание для языка игры.
pub fn available(lang: Lang) -> Result<(), String> {
    create_engine(lang).map(|_| ())
}

pub struct OcrWorker {
    stop: Arc<AtomicBool>,
}

impl OcrWorker {
    pub fn start(
        pid: u32,
        lang: Lang,
        region: Arc<dyn Fn() -> Option<(i32, i32, i32, i32)> + Send + Sync>,
        events: Sender<HookEvent>,
        waker: Waker,
    ) -> OcrWorker {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let _ = std::thread::Builder::new().name("ocr".into()).spawn(move || {
            run(pid, lang, region, events, waker, flag)
        });
        OcrWorker { stop }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for OcrWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(
    pid: u32,
    lang: Lang,
    region: Arc<dyn Fn() -> Option<(i32, i32, i32, i32)> + Send + Sync>,
    events: Sender<HookEvent>,
    waker: Waker,
    stop: Arc<AtomicBool>,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let engine = match create_engine(lang) {
        Ok(e) => e,
        Err(msg) => {
            let _ = events.send(HookEvent::AttachFailed(pid, msg));
            waker();
            return;
        }
    };
    let max_dim = OcrEngine::MaxImageDimension().unwrap_or(2600) as i32;
    let _ = events.send(HookEvent::Connected(pid));
    let _ = events.send(HookEvent::ThreadCreated(thread_info(pid)));
    waker();

    let (mut prev_hash, mut done_hash, mut last_text) = (None::<u64>, None::<u64>, String::new());
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(400));
        let Some((x, y, w, h)) = region() else { continue };
        // область больше предела движка — обрезаем до предела (правый/нижний край теряется)
        let (w, h) = (w.min(max_dim), h.min(max_dim));
        let Some(pixels) = capture(x, y, w, h) else { continue };
        let mut hs = DefaultHasher::new();
        pixels.hash(&mut hs);
        let hash = hs.finish();
        let stable = prev_hash == Some(hash);
        prev_hash = Some(hash);
        if !stable || done_hash == Some(hash) {
            continue;
        }
        done_hash = Some(hash);
        let Ok(raw) = recognize(&engine, &pixels, w, h) else { continue };
        let text = tidy(&raw);
        if !text.is_empty() && text != last_text {
            last_text = text.clone();
            let _ = events.send(HookEvent::Text(thread_info(pid), text));
            waker();
        }
    }
}

/// Снимок области экрана: BGRA, сверху вниз, альфа = 255.
fn capture(x: i32, y: i32, w: i32, h: i32) -> Option<Vec<u8>> {
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(HWND::default());
        let dc = CreateCompatibleDC(screen);
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
        let bmp = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
        let mut out = None;
        if let Ok(bmp) = bmp {
            if !bits.is_null() {
                let old = SelectObject(dc, bmp);
                if BitBlt(dc, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT).is_ok() {
                    let mut v = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
                    v.chunks_exact_mut(4).for_each(|p| p[3] = 255);
                    out = Some(v);
                }
                SelectObject(dc, old);
            }
            let _ = DeleteObject(bmp);
        }
        let _ = DeleteDC(dc);
        ReleaseDC(HWND::default(), screen);
        out
    }
}

fn recognize(engine: &OcrEngine, pixels: &[u8], w: i32, h: i32) -> windows::core::Result<String> {
    let writer = DataWriter::new()?;
    writer.WriteBytes(pixels)?;
    let buffer = writer.DetachBuffer()?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, w, h)?;
    let result = engine.RecognizeAsync(&bitmap)?.get()?;
    let mut lines = Vec::new();
    for line in result.Lines()? {
        lines.push(line.Text()?.to_string());
    }
    Ok(lines.join("\n"))
}

/// Windows OCR для японского/китайского вставляет пробелы между знаками и рвёт реплику на строки.
fn tidy(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let (Some(a), Some(b)) = (out.chars().last(), line.chars().next()) {
            if a.is_ascii() || b.is_ascii() {
                out.push(' ');
            }
        }
        out.push_str(line);
    }
    let chars: Vec<char> = out.chars().collect();
    let mut res = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ' ' {
            let mut j = i;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            let between_wide = i > 0 && j < chars.len() && !chars[i - 1].is_ascii() && !chars[j].is_ascii();
            if !between_wide {
                res.push(' ');
            }
            i = j;
        } else {
            res.push(c);
            i += 1;
        }
    }
    res.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_works() {
        assert_eq!(
            tidy("「 お は よ う ご ざ い ま す 。\n今 日 も 早 い で す ね 」"),
            "「おはようございます。今日も早いですね」"
        );
        assert_eq!(tidy("Hello  world\nagain"), "Hello world again");
        assert_eq!(tidy("  "), "");
    }

    #[test]
    #[ignore]
    fn live_recognize() {
        let engine = create_engine(Lang::Ja).unwrap();
        let px = capture(0, 0, 400, 100).unwrap();
        println!("{:?}", recognize(&engine, &px, 400, 100).map(|s| tidy(&s)));
    }
}
