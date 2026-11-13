//! Демо-источник текста (--demo): имитация игры и потоков LunaHook, чтобы смотреть окна без игры.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use noveltl_core::langdetect::Lang;

use crate::source::{HookEvent, HookSource, ThreadInfo, Waker};

pub struct ScriptLine {
    pub name: &'static str,
    pub text: &'static str,
    pub name_ru: &'static str,
    pub text_ru: &'static str,
    pub name_en: &'static str,
    pub text_en: &'static str,
}

/// Собственный текст для демонстрации.
pub const SCRIPT: &[ScriptLine] = &[
    ScriptLine { name: "", text: "朝の境内は静かで、空気がひんやりと澄んでいた。", name_ru: "", text_ru: "Утром на территории храма было тихо, а воздух был прохладным и прозрачным.", name_en: "", text_en: "The shrine grounds were quiet in the morning, and the air was cool and clear." },
    ScriptLine { name: "灯里", text: "「おはようございます。今日も早いですね」", name_ru: "Акари", text_ru: "«Доброе утро. Вы сегодня снова рано».", name_en: "Akari", text_en: "\"Good morning. You're up early again today.\"" },
    ScriptLine { name: "悠斗", text: "「ああ、おはよう。朝の掃除を手伝おうと思って」", name_ru: "Юто", text_ru: "«А, доброе утро. Я подумал, что помогу с утренней уборкой».", name_en: "Yuto", text_en: "\"Oh, good morning. I thought I'd help with the morning cleaning.\"" },
    ScriptLine { name: "灯里", text: "「ふふ、ありがとうございます。それでは、箒をお願いしますね」", name_ru: "Акари", text_ru: "«Хе-хе, спасибо большое. Тогда возьмите, пожалуйста, метлу».", name_en: "Akari", text_en: "\"Hehe, thank you so much. Then please take the broom.\"" },
    ScriptLine { name: "", text: "彼女は小さく微笑んで、箒を差し出した。", name_ru: "", text_ru: "Она слегка улыбнулась и протянула метлу.", name_en: "", text_en: "She smiled faintly and held out the broom." },
    ScriptLine { name: "小春", text: "「お二人とも、朝ごはんの用意ができましたよ」", name_ru: "Кохару", text_ru: "«Вы оба, завтрак уже готов».", name_en: "Koharu", text_en: "\"You two, breakfast is ready.\"" },
    ScriptLine { name: "悠斗", text: "「もうそんな時間か。すぐ行くよ」", name_ru: "Юто", text_ru: "«Уже так поздно? Сейчас приду».", name_en: "Yuto", text_en: "\"Is it that time already? I'll be right there.\"" },
    ScriptLine { name: "灯里", text: "「今日の朝ごはんは何でしょう。楽しみです」", name_ru: "Акари", text_ru: "«Интересно, что сегодня на завтрак. Жду с нетерпением».", name_en: "Akari", text_en: "\"I wonder what's for breakfast today. I can't wait.\"" },
];

pub const DEMO_TITLE: &str = "Kirinomori Lantern (NovelTL demo)";
const TITLE_TEXT: &str = "Kirinomori Lantern";

// (код, имя, addr, ctx, ctx2)
type Hook = (&'static str, &'static str, u64, u64, u64);
const TEXT_HOOK: Hook = ("HQ-1C*0@0:kirikiri.exe", "KiriKiriZ", 0x7FF6A0012340, 0x7FF6A0AB0010, 0x1);
const NAME_HOOK: Hook = ("HQ-1C*0@0:kirikiri.exe", "KiriKiriZ", 0x7FF6A0012340, 0x7FF6A0AB0020, 0x2);
const FILE_HOOK: Hook = ("HS-8@0:kernel32.dll:CreateFileW", "KiriKiriZ_File", 0x7FF6A0055000, 0x7FF6A0CC0030, 0x0);
const GLYPH_HOOK: Hook = ("HQ8@0:gdi32.dll:GetGlyphOutlineW", "GetGlyphOutlineW", 0x7FFB10020000, 0x7FF6A0DD0040, 0x0);
const TITLE_HOOK: Hook = ("HQ-C@0:gdi32.dll:TextOutW", "TextOutW", 0x7FFB10030000, 0x7FF6A0EE0050, 0x0);
const FILES: [&str; 5] = [
    "bg_keidai_morning.png",
    "voice\\yos_0012.ogg",
    "scenario\\001_prologue.ks",
    "se_broom_01.ogg",
    "bgm_03.ogg",
];

fn info(hook: Hook, pid: u32) -> ThreadInfo {
    ThreadInfo {
        code: hook.0.to_string(),
        name: hook.1.to_string(),
        pid,
        addr: hook.2,
        ctx: hook.3,
        ctx2: hook.4,
    }
}

fn emit(events: &Sender<HookEvent>, waker: &Waker, ev: HookEvent) {
    let _ = events.send(ev);
    waker();
}

/// Выдаёт те же события, что и LunaHost: реплики, имена и разный мусор.
#[derive(Clone)]
pub struct DemoSource {
    events: Sender<HookEvent>,
    waker: Waker,
    pid: u32,
    attached: Arc<AtomicBool>,
}

impl DemoSource {
    pub fn new(events: Sender<HookEvent>, waker: Waker) -> Self {
        DemoSource { events, waker, pid: std::process::id(), attached: Arc::new(AtomicBool::new(false)) }
    }

    /// «Игра» показала реплику `index`: шлём её так, как это сделали бы хуки.
    pub fn emit_line(&self, index: usize) {
        if !self.attached.load(Ordering::SeqCst) {
            return;
        }
        let line = &SCRIPT[index % SCRIPT.len()];
        let (name, text) = (line.name, line.text);
        // движок: имя в 【】, теги KiriKiri и перенос посередине
        let chars: Vec<char> = text.chars().collect();
        let half = chars.len() / 2;
        let first: String = chars[..half].iter().collect();
        let second: String = chars[half..].iter().collect();
        let mut raw = String::new();
        if !name.is_empty() {
            raw.push_str(&format!("【{name}】"));
        }
        raw.push_str("%f$ll;");
        raw.push_str(&first);
        raw.push_str("[r]");
        raw.push_str(&second);
        raw.push_str("[p]");
        let (ev, wk, pid) = (&self.events, &self.waker, self.pid);
        emit(ev, wk, HookEvent::Text(info(TEXT_HOOK, pid), raw));
        if !name.is_empty() {
            emit(ev, wk, HookEvent::Text(info(NAME_HOOK, pid), name.to_string()));
        }
        emit(ev, wk, HookEvent::Text(info(FILE_HOOK, pid), FILES[index % FILES.len()].to_string()));
        emit(ev, wk, HookEvent::Text(info(TITLE_HOOK, pid), TITLE_TEXT.to_string()));
        // хук рисования: по одному символу, много строк в секунду
        let glyphs: Vec<char> = chars.into_iter().take(14).collect();
        let (ev, wk, attached) = (self.events.clone(), self.waker.clone(), self.attached.clone());
        std::thread::spawn(move || {
            for (i, ch) in glyphs.into_iter().enumerate() {
                if i > 0 {
                    std::thread::sleep(Duration::from_millis(20));
                }
                if !attached.load(Ordering::SeqCst) {
                    return;
                }
                let s: String = [ch, ch].iter().collect();
                emit(&ev, &wk, HookEvent::Text(info(GLYPH_HOOK, pid), s));
            }
        });
    }
}

impl HookSource for DemoSource {
    fn attach(&mut self, _pid: u32, _codepage: u32) {
        let (ev, wk, attached, pid) = (self.events.clone(), self.waker.clone(), self.attached.clone(), self.pid);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            attached.store(true, Ordering::SeqCst);
            emit(&ev, &wk, HookEvent::Connected(pid));
            emit(&ev, &wk, HookEvent::HostInfo(4, "KiriKiriZ".to_string()));
            emit(&ev, &wk, HookEvent::Text(info(TITLE_HOOK, pid), TITLE_TEXT.to_string()));
        });
    }

    fn detach(&mut self, _pid: u32) {
        if self.attached.swap(false, Ordering::SeqCst) {
            emit(&self.events, &self.waker, HookEvent::Disconnected(self.pid));
        }
    }


    fn shutdown(&mut self) {
        self.attached.store(false, Ordering::SeqCst);
    }

    fn error(&self) -> String {
        String::new()
    }
}

/// Готовые переводы сценария: текст -> перевод (демо работает без интернета).
pub fn demo_translations() -> HashMap<&'static str, &'static str> {
    demo_translations_for(Lang::Ru)
}

/// То же для выбранного языка перевода: английский — свой, для остальных целей запасной вариант — английский
/// (русский только для Ru).
pub fn demo_translations_for(target: Lang) -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    let ru = target == Lang::Ru;
    for l in SCRIPT {
        m.insert(l.text, if ru { l.text_ru } else { l.text_en });
        if !l.name.is_empty() {
            m.insert(l.name, if ru { l.name_ru } else { l.name_en });
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn collect(rx: &std::sync::mpsc::Receiver<HookEvent>, ms: u64) -> Vec<HookEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.recv_timeout(Duration::from_millis(ms)) {
            out.push(ev);
        }
        out
    }

    #[test]
    fn script_has_eight_lines() {
        assert_eq!(SCRIPT.len(), 8);
        assert_eq!(demo_translations().get("「おはようございます。今日も早いですね」"), Some(&"«Доброе утро. Вы сегодня снова рано»."));
        assert_eq!(demo_translations().get("灯里"), Some(&"Акари"));
    }

    #[test]
    fn attach_and_emit_line() {
        let (tx, rx) = channel();
        let waker: Waker = Arc::new(|| {});
        let mut src = DemoSource::new(tx, waker);
        src.emit_line(1); // до attach — тишина
        assert!(rx.try_recv().is_err());
        src.attach(0, 932);
        let first = collect(&rx, 1500);
        assert!(matches!(first.first(), Some(HookEvent::Connected(_))));
        assert!(first.iter().any(|e| matches!(e, HookEvent::HostInfo(4, n) if n == "KiriKiriZ")));

        src.emit_line(1);
        let evs = collect(&rx, 300);
        let main = evs.iter().find_map(|e| match e {
            HookEvent::Text(t, s) if t.code == TEXT_HOOK.0 && t.ctx == TEXT_HOOK.3 => Some(s.clone()),
            _ => None,
        });
        let main = main.expect("нет основного хука");
        assert!(main.contains("【灯里】"));
        assert!(main.contains("[r]"));
        assert!(evs.iter().any(|e| matches!(e, HookEvent::Text(t, s) if t.ctx == NAME_HOOK.3 && s == "灯里")));
        assert!(evs.iter().any(|e| matches!(e, HookEvent::Text(t, _) if t.ctx == GLYPH_HOOK.3)));

        src.detach(0);
        assert!(matches!(collect(&rx, 100).last(), Some(HookEvent::Disconnected(_))));
    }
}
