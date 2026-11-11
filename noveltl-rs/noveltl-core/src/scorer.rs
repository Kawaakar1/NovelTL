//! Автоподбор хука: оценка текстовых потоков LunaHook.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::cleaner::{clean, CleanResult};
use crate::langdetect::is_cjk;

/// (код хука, ctx & 0xFFFF, ctx2 & 0xFFFF)
pub type HookKey = (String, u16, u16);

/// textrender.dll — плагин вывода текста KiriKiri
pub const ENGINE_MARKERS: &[&str] = &["kirikiri", "krkr", "textrender"];
pub const GENERIC_MARKERS: &[&str] = &[
    "getglyphoutline", "textout", "drawtext", "exttextout",
    // прочие общие функции Windows, которые LunaHook перехватывает в любой программе
    "gettextextent", "getcharabcwidths", "getcharwidth", "getglyphindices", "getcharacterplacement",
    "lstr", "multibytetowidechar", "widechartomultibyte", "getstringtype", "foldstring",
    "charnext", "charprev", "wvsprintf",
];

pub const CONFIDENT_SCORE: f64 = 55.0;
pub const CONFIDENT_MARGIN: f64 = 8.0;
pub const CONFIDENT_DISTINCT: usize = 3;
pub const NAME_WINDOW: f64 = 1.0; // секунд между именем и репликой (поиск потока имён)
/// Имя считается именем ЭТОЙ реплики, только если пришло почти одновременно с ней:
/// иначе при быстром пролистывании реплика рассказчика получала бы имя предыдущей.
pub const NAME_PAIR_WINDOW: f64 = 0.35;
pub const MAX_LINES: usize = 120;

static RE_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\.(ks|tjs|png|jpe?g|bmp|tlg|ogg|wav|mp3|mp4|txt|dll|exe|xp3|scn|psb|ini|json|csv|pimg|mpg|wmv|otf|ttf)\b",
    )
    .unwrap()
});
static RE_DRIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z]:[\\/]").unwrap());
static RE_IDENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_\-.:#@$]+$").unwrap());
static RE_CAMEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z][A-Z]").unwrap());

const SPEECH_PUNCT: &str = "。、！？!?…「」『』（）()—―-~〜・,.'\"“”‘’:;♪　 ";
const SENTENCE_MARKS: &str = "。！？!?…「」『』.,、";
const NAME_END: &str = "。.!?！？…";

/// Ключ хука, устойчивый между запусками: адреса меняются, младшие 16 бит — нет.
pub fn make_key(code: &str, ctx: u64, ctx2: u64) -> HookKey {
    (code.to_string(), (ctx & 0xFFFF) as u16, (ctx2 & 0xFFFF) as u16)
}

/// Сохранённый в профиле хук.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookRef {
    pub code: String,
    pub name: String,
    pub ctx_lo: u16,
    pub ctx2_lo: u16,
}

impl HookRef {
    pub fn from_thread(code: &str, name: &str, ctx: u64, ctx2: u64) -> Self {
        HookRef {
            code: code.to_string(),
            name: name.to_string(),
            ctx_lo: (ctx & 0xFFFF) as u16,
            ctx2_lo: (ctx2 & 0xFFFF) as u16,
        }
    }

    pub fn key(&self) -> HookKey {
        (self.code.clone(), self.ctx_lo, self.ctx2_lo)
    }
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn fmean(it: impl Iterator<Item = f64>) -> f64 {
    let (mut sum, mut n) = (0.0, 0usize);
    for v in it {
        sum += v;
        n += 1;
    }
    if n == 0 {
        0.0
    } else {
        sum / n as f64
    }
}

fn median(mut v: Vec<usize>) -> f64 {
    v.sort_unstable();
    let n = v.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        v[n / 2] as f64
    } else {
        (v[n / 2 - 1] + v[n / 2]) as f64 / 2.0
    }
}

/// 0..1 — насколько строка похожа на реплику, а не на путь или идентификатор.
pub fn speech_likeness(text: &str) -> f64 {
    let s = text.trim();
    if s.is_empty() {
        return 0.0;
    }
    if RE_FILE.is_match(s) || RE_DRIVE.is_match(s) {
        return 0.0;
    }
    let ascii_only = s.is_ascii();
    if ascii_only && (s.contains('\\') || s.contains('/')) {
        return 0.0;
    }
    if ascii_only && !s.contains(' ') && RE_IDENT.is_match(s) {
        // s[:-1].rstrip(".")
        let mut head = s.chars();
        head.next_back();
        let head = head.as_str().trim_end_matches('.');
        if s.contains('_')
            || s.chars().any(|c| c.is_ascii_digit())
            || RE_CAMEL.is_match(s)
            || head.contains('.')
        {
            return 0.0;
        }
        return 0.5;
    }
    let mut good = 0usize;
    let mut total = 0usize;
    for ch in s.chars() {
        total += 1;
        if is_cjk(ch) || ch.is_alphabetic() || SPEECH_PUNCT.contains(ch) {
            good += 1;
        }
    }
    let mut ratio = good as f64 / total as f64;
    if s.chars().any(|ch| SENTENCE_MARKS.contains(ch)) {
        ratio += 0.15;
    }
    if s.contains('\u{FFFD}') {
        ratio -= 0.5;
    }
    ratio.min(1.0).max(0.0)
}

/// Один текстовый поток (хук) и его последние строки.
#[derive(Clone, Debug)]
pub struct ThreadStats {
    pub hookcode: String,
    pub hookname: String,
    pub addr: u64,
    pub ctx: u64,
    pub ctx2: u64,
    pub total: usize,
    pub lines: VecDeque<(f64, String)>, // (время, сырой текст)
    /// Результат clean() для каждой строки из `lines` (считается один раз при добавлении).
    pub cleaned: VecDeque<CleanResult>,
}

impl ThreadStats {
    pub fn new(hookcode: &str, hookname: &str, addr: u64, ctx: u64, ctx2: u64) -> Self {
        ThreadStats {
            hookcode: hookcode.to_string(),
            hookname: hookname.to_string(),
            addr,
            ctx,
            ctx2,
            total: 0,
            lines: VecDeque::new(),
            cleaned: VecDeque::new(),
        }
    }

    pub fn key(&self) -> HookKey {
        make_key(&self.hookcode, self.ctx, self.ctx2)
    }

    pub fn add(&mut self, text: &str, now: Option<f64>) {
        self.total += 1;
        if self.lines.len() >= MAX_LINES {
            self.lines.pop_front();
            self.cleaned.pop_front();
        }
        self.lines.push_back((now.unwrap_or_else(now_secs), text.to_string()));
        self.cleaned.push_back(clean(text));
    }

    pub fn last_raw(&self, n: usize) -> Vec<String> {
        let skip = self.lines.len().saturating_sub(n);
        self.lines.iter().skip(skip).map(|(_, t)| t.clone()).collect()
    }

    pub fn last_clean(&self, n: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if n == 0 {
            return out;
        }
        for res in self.cleaned.iter().rev() {
            if res.text.is_empty() {
                continue;
            }
            let line = if res.name.is_empty() {
                res.text.clone()
            } else {
                format!("{}: {}", res.name, res.text)
            };
            if out.last().map_or(true, |l| *l != line) {
                out.push(line);
            }
            if out.len() >= n {
                break;
            }
        }
        out.reverse();
        out
    }
}

#[derive(Clone, Debug)]
pub struct ScoreResult {
    pub key: HookKey,
    pub hookname: String,
    pub total: usize,
    pub score: f64,
    pub distinct: usize,
    pub details: Vec<(String, f64)>,
}

/// Максимум строк за любую секунду.
fn peak_rate(times: &[f64]) -> usize {
    let (mut peak, mut start) = (0usize, 0usize);
    for end in 0..times.len() {
        while times[end] - times[start] > 1.0 {
            start += 1;
        }
        peak = peak.max(end - start + 1);
    }
    peak
}

pub fn score_thread(st: &ThreadStats) -> ScoreResult {
    let cleaned: Vec<&str> = st
        .cleaned
        .iter()
        .map(|c| c.text.as_str())
        .filter(|c| !c.is_empty())
        .collect();
    let name = st.hookname.to_lowercase();
    let mk = |score: f64, distinct: usize, details: Vec<(String, f64)>| ScoreResult {
        key: st.key(),
        hookname: st.hookname.clone(),
        total: st.total,
        score,
        distinct,
        details,
    };
    if cleaned.is_empty() {
        return mk(0.0, 0, vec![("пусто".to_string(), 1.0)]);
    }

    let mut details: Vec<(String, f64)> = Vec::new();
    let distinct = cleaned.iter().collect::<HashSet<_>>().len();
    details.push(("разных_реплик".into(), round1(distinct.min(4) as f64 / 4.0 * 30.0)));

    let speech = fmean(cleaned.iter().map(|c| speech_likeness(c)));
    details.push(("похоже_на_речь".into(), round1(speech * 35.0)));

    let times: Vec<f64> = st.lines.iter().map(|(t, _)| *t).collect();
    let peak = peak_rate(&times);
    let mut spam = 0.0;
    if peak >= 3 {
        spam -= 40.0f64.min((peak as f64 - 2.0) * 10.0);
    }
    if cleaned.len() >= 4 {
        // одни и те же строки снова и снова — меню, подписи кнопок, имена
        let ratio = distinct as f64 / cleaned.len() as f64;
        if ratio < 0.75 {
            spam -= 30.0f64.min(round1((0.75 - ratio) * 60.0));
        }
    }
    details.push(("спам".into(), spam));

    let median_len = median(cleaned.iter().map(|c| c.chars().count()).collect());
    let length = if median_len < 2.0 {
        -15.0
    } else if median_len < 6.0 {
        5.0
    } else if median_len <= 150.0 {
        15.0
    } else if median_len <= 300.0 {
        0.0
    } else {
        -10.0
    };
    details.push(("длина".into(), length));

    let mut bonus = 0.0;
    if ENGINE_MARKERS.iter().any(|m| name.contains(m)) {
        bonus += 12.0;
    }
    if GENERIC_MARKERS.iter().any(|m| name.contains(m)) {
        bonus -= 12.0;
    }
    details.push(("тип_хука".into(), bonus));

    let score: f64 = details.iter().map(|(_, v)| *v).sum();
    mk(round1(score.max(0.0)), distinct, details)
}

pub fn rank_threads<'a>(threads: impl IntoIterator<Item = &'a ThreadStats>) -> Vec<ScoreResult> {
    let mut results: Vec<ScoreResult> = threads
        .into_iter()
        .map(score_thread)
        .filter(|r| r.score > 0.0)
        .collect();
    // стабильная сортировка по убыванию (score, total)
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then(b.total.cmp(&a.total))
    });
    results
}

pub fn is_confident(ranked: &[ScoreResult]) -> bool {
    let Some(best) = ranked.first() else {
        return false;
    };
    if best.score < CONFIDENT_SCORE || best.distinct < CONFIDENT_DISTINCT {
        return false;
    }
    ranked.len() == 1 || best.score - ranked[1].score >= CONFIDENT_MARGIN
}

/// Ищет поток с именами говорящих: короткие строки, приходящие вместе с репликами.
pub fn find_name_thread(threads: &ThreadTracker, dialog_key: &HookKey) -> Option<HookKey> {
    let dialog = threads.get(dialog_key)?;
    if dialog.lines.len() < 3 {
        return None;
    }
    let dialog_times: Vec<f64> = dialog.lines.iter().map(|(t, _)| *t).collect();
    let mut best: Option<HookKey> = None;
    let mut best_rank: Option<(f64, usize)> = None;
    for st in threads.iter() {
        let key = st.key();
        if &key == dialog_key || st.lines.len() < 3 {
            continue;
        }
        let times: Vec<f64> = st.lines.iter().map(|(t, _)| *t).collect();
        if peak_rate(&times) >= 4 {
            continue;
        }
        let entries: Vec<(f64, &str)> = st
            .lines
            .iter()
            .zip(st.cleaned.iter())
            .map(|((t, _), c)| (*t, c.text.as_str()))
            .filter(|(_, c)| !c.is_empty())
            .collect();
        if entries.len() < 3 {
            continue;
        }
        let texts: Vec<&str> = entries.iter().map(|(_, c)| *c).collect();
        if texts.iter().collect::<HashSet<_>>().len() < 2 {
            continue; // одна и та же строка (заголовок окна и т. п.) — это не имена
        }
        if median(texts.iter().map(|c| c.chars().count()).collect()) > 24.0 {
            continue;
        }
        let ends = texts
            .iter()
            .filter(|c| c.chars().last().is_some_and(|ch| NAME_END.contains(ch)))
            .count();
        if ends as f64 / texts.len() as f64 > 0.2 {
            continue;
        }
        if fmean(texts.iter().map(|c| speech_likeness(c))) < 0.5 {
            continue;
        }
        let near = entries
            .iter()
            .filter(|(t, _)| dialog_times.iter().any(|d| (t - d).abs() <= NAME_WINDOW))
            .count();
        let share = near as f64 / entries.len() as f64;
        if share < 0.7 {
            continue;
        }
        let rank = ((share * 100.0).round() / 100.0, entries.len());
        let better = match best_rank {
            None => true,
            Some((bs, bn)) => match rank.0.partial_cmp(&bs) {
                Some(Ordering::Greater) => true,
                Some(Ordering::Equal) => rank.1 > bn,
                _ => false,
            },
        };
        if better {
            best = Some(key);
            best_rank = Some(rank);
        }
    }
    best
}

/// Все потоки текущей игры. Потоки с одинаковым ключом объединяются.
#[derive(Default)]
pub struct ThreadTracker {
    list: Vec<ThreadStats>, // в порядке появления
    index: HashMap<HookKey, usize>,
    /// Кэш ranked(): рейтинг не зависит от текущего времени; сбрасывается при любом изменении потоков.
    ranked_cache: std::sync::Mutex<Option<Vec<ScoreResult>>>,
}

impl ThreadTracker {
    fn invalidate(&mut self) {
        *self.ranked_cache.get_mut().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn clear(&mut self) {
        self.invalidate();
        self.list.clear();
        self.index.clear();
    }

    pub fn ensure(&mut self, hookcode: &str, hookname: &str, addr: u64, ctx: u64, ctx2: u64) -> &mut ThreadStats {
        // через &mut ThreadStats можно добавить текст — кэш рейтинга устарел
        self.invalidate();
        let key = make_key(hookcode, ctx, ctx2);
        let idx = match self.index.get(&key) {
            Some(&i) => i,
            None => {
                self.list.push(ThreadStats::new(hookcode, hookname, addr, ctx, ctx2));
                self.index.insert(key, self.list.len() - 1);
                self.list.len() - 1
            }
        };
        &mut self.list[idx]
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_text(
        &mut self,
        hookcode: &str,
        hookname: &str,
        addr: u64,
        ctx: u64,
        ctx2: u64,
        text: &str,
        now: Option<f64>,
    ) -> HookKey {
        let st = self.ensure(hookcode, hookname, addr, ctx, ctx2);
        st.add(text, now);
        st.key()
    }

    pub fn get(&self, key: &HookKey) -> Option<&ThreadStats> {
        self.index.get(key).map(|&i| &self.list[i])
    }

    pub fn iter(&self) -> impl Iterator<Item = &ThreadStats> {
        self.list.iter()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn ranked(&self) -> Vec<ScoreResult> {
        let mut cache = self.ranked_cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.get_or_insert_with(|| rank_threads(self.list.iter())).clone()
    }

    /// Ищет сохранённый хук среди живых потоков.
    pub fn find_saved(&self, saved: Option<&HookRef>) -> Option<HookKey> {
        let key = saved?.key();
        self.index.contains_key(&key).then_some(key)
    }

    pub fn auto_pick(&self) -> Option<ScoreResult> {
        let ranked = self.ranked();
        if is_confident(&ranked) {
            ranked.into_iter().next()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIALOG: [&str; 4] = [
        "【芳乃】「おはようございます、有地さん」",
        "【将臣】「ああ、おはよう。今日も早いな」",
        "朝の空気は澄んでいて、気持ちがいい。",
        "【芳乃】「朝のお勤めがありますから」",
    ];

    fn feed(t: &mut ThreadTracker, code: &str, name: &str, ctx: u64, lines: &[&str], start: f64, step: f64) {
        for (i, line) in lines.iter().enumerate() {
            t.add_text(code, name, 0x1000, ctx, 0, line, Some(start + i as f64 * step));
        }
    }

    fn hookcode_of(r: &ScoreResult) -> &str {
        &r.key.0
    }

    #[test]
    fn speech_likeness_cases() {
        assert!(speech_likeness("「おはようございます」") > 0.9);
        assert!(speech_likeness("Good morning, everyone.") > 0.9);
        assert_eq!(speech_likeness("data\\scenario\\001.ks"), 0.0);
        assert_eq!(speech_likeness("bg_school_01"), 0.0);
        assert_eq!(speech_likeness("C:/Games/save.dat"), 0.0);
        assert_eq!(speech_likeness("evCharaShow"), 0.0);
    }

    #[test]
    fn make_key_uses_low_16_bits() {
        let a = make_key("HQ-1C@0:x.exe", 0x7FF612345678, 0xABCD0001);
        let b = make_key("HQ-1C@0:x.exe", 0x7FF699995678, 0x11110001);
        assert_eq!(a, b);
        assert_eq!(a, ("HQ-1C@0:x.exe".to_string(), 0x5678, 0x0001));
    }

    #[test]
    fn dialog_beats_garbage() {
        let mut t = ThreadTracker::default();
        feed(&mut t, "HKIRI", "KiriKiriZ", 0x10, &DIALOG, 0.0, 2.0);
        // спам по одному символу — много строк в секунду
        let chars: Vec<String> = "おはようございます有地さん".chars().map(|c| c.to_string()).collect();
        let refs: Vec<&str> = chars.iter().map(|s| s.as_str()).collect();
        feed(&mut t, "HGLYPH", "GetGlyphOutlineW", 0x20, &refs, 0.0, 0.05);
        feed(
            &mut t,
            "HPATH",
            "KiriKiriZ",
            0x30,
            &["bg_001.png", "se_door.ogg", "scenario\\001.ks", "bgm_02.ogg"],
            0.0,
            2.0,
        );
        let ranked = t.ranked();
        assert_eq!(hookcode_of(&ranked[0]), "HKIRI");
        assert!(is_confident(&ranked));
        assert_eq!(hookcode_of(&t.auto_pick().unwrap()), "HKIRI");
    }

    #[test]
    fn engine_hook_preferred_over_generic() {
        let mut t = ThreadTracker::default();
        feed(&mut t, "HKIRI", "KiriKiriZ", 0x10, &DIALOG, 0.0, 2.0);
        feed(&mut t, "HTEXT", "TextOutW", 0x20, &DIALOG, 0.0, 2.0);
        let ranked = t.ranked();
        assert_eq!(hookcode_of(&ranked[0]), "HKIRI");
        assert!(ranked[0].score - ranked[1].score >= 20.0);
    }

    #[test]
    fn dialog_beats_names_only() {
        let mut t = ThreadTracker::default();
        feed(&mut t, "HKIRI", "KiriKiriZ", 0x10, &DIALOG, 0.0, 2.0);
        feed(&mut t, "HNAME", "KiriKiriZ", 0x20, &["芳乃", "将臣", "芳乃", "芳乃"], 0.0, 2.0);
        assert_eq!(hookcode_of(&t.ranked()[0]), "HKIRI");
    }

    #[test]
    fn not_confident_with_few_lines() {
        let mut t = ThreadTracker::default();
        feed(&mut t, "HKIRI", "KiriKiriZ", 0x10, &DIALOG[..2], 0.0, 2.0);
        assert!(t.auto_pick().is_none());
    }

    #[test]
    fn spam_penalty() {
        let mut t = ThreadTracker::default();
        feed(&mut t, "A", "x", 1, &DIALOG, 0.0, 2.0);
        feed(&mut t, "B", "x", 2, &DIALOG, 0.0, 0.01);
        let scores: HashMap<String, f64> = t.ranked().iter().map(|r| (r.key.0.clone(), r.score)).collect();
        assert!(scores["A"] > scores["B"]);
    }

    #[test]
    fn find_saved_hook_across_runs() {
        let mut t = ThreadTracker::default();
        feed(&mut t, "HKIRI", "KiriKiriZ", 0x7FF600001234, &DIALOG, 0.0, 2.0);
        let saved = HookRef { code: "HKIRI".into(), name: "KiriKiriZ".into(), ctx_lo: 0x1234, ctx2_lo: 0 };
        assert_eq!(t.find_saved(Some(&saved)), Some(("HKIRI".to_string(), 0x1234, 0)));
        let other = HookRef { code: "OTHER".into(), name: String::new(), ctx_lo: 0x1234, ctx2_lo: 0 };
        assert_eq!(t.find_saved(Some(&other)), None);
        assert_eq!(t.find_saved(None), None);
    }

    #[test]
    fn empty_thread_scores_zero() {
        let mut t = ThreadTracker::default();
        let st = t.ensure("H", "n", 0, 0, 0).clone();
        let r = score_thread(&st);
        assert_eq!(r.score, 0.0);
        assert_eq!(r.details, vec![("пусто".to_string(), 1.0)]);
    }

    #[test]
    fn hookref_json_roundtrip() {
        let r = HookRef::from_thread("H", "n", 0x12345, 0x6789A);
        assert_eq!((r.ctx_lo, r.ctx2_lo), (0x2345, 0x789A));
        let js = serde_json::to_string(&r).unwrap();
        let back: HookRef = serde_json::from_str(&js).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn last_clean_zero_and_cached_ranking() {
        let mut st = ThreadStats::new("H", "n", 0, 0, 0);
        st.add("こんにちは", Some(1.0));
        assert!(st.last_clean(0).is_empty());
        assert_eq!(st.last_clean(1), vec!["こんにちは".to_string()]);
        assert_eq!(st.cleaned.len(), st.lines.len());
        // кэш рейтинга сбрасывается при новом тексте
        let mut t = ThreadTracker::default();
        feed(&mut t, "A", "x", 1, &DIALOG[..2], 0.0, 2.0);
        let before = t.ranked()[0].total;
        assert_eq!(t.ranked()[0].total, before);
        feed(&mut t, "A", "x", 1, &DIALOG[2..], 10.0, 2.0);
        assert_eq!(t.ranked()[0].total, before + 2);
        t.clear();
        assert!(t.ranked().is_empty());
    }

    #[test]
    fn lines_are_capped() {
        let mut st = ThreadStats::new("H", "n", 0, 0, 0);
        for i in 0..(MAX_LINES + 5) {
            st.add(&format!("line {i}"), Some(i as f64));
        }
        assert_eq!(st.lines.len(), MAX_LINES);
        assert_eq!(st.total, MAX_LINES + 5);
        assert_eq!(st.last_raw(1), vec![format!("line {}", MAX_LINES + 4)]);
    }

    // --- по отчёту с настоящей Senren*Banka (англ. Steam-версия) ---
    const REAL_DIALOG: [&str; 7] = [
        "First of all, I don't live here.",
        "Ah! My apologies, I forgot to introduce myself. My name is Hitachi Mako.",
        "N-Nice to meet you. I'm Arichi Masaomi.",
        "I have been serving the Tomotake family for a long time now.",
        "What do you mean, serving?",
        "Huh, I see.",
        "So it's an old tradition.",
    ];
    const REAL_NAMES: [&str; 6] = [
        "Mako  Mako  Mako  ",
        "Mako  Mako  Mako  ",
        "Masaomi  Masaomi  Masaomi  ",
        "Mako  Mako  Mako  ",
        "Masaomi  Masaomi  Masaomi  ",
        "Mako  Mako  Mako  ",
    ];
    const REAL_UI: [&str; 8] = [
        "0001This is the load interface. Please choose a save file to load.Load 0001-01.Load 0001-01.",
        "Settings",
        "Replay voice",
        "Replay voice",
        "Settings",
        "Settings",
        "Settings",
        "Skip modeNext sceneNext choice",
    ];
    const REAL_CODE: &str = "ENHQXN-4@C3B1:textrender.dll";
    const REAL_HW: &str = "HW-8*14:-8*0@F4600:SenrenBanka.exe";

    fn real_tracker() -> ThreadTracker {
        let mut t = ThreadTracker::default();
        feed(&mut t, REAL_CODE, "TextRender", 0, &REAL_DIALOG, 10.0, 3.0);
        // имена приходят вместе с репликами (кроме последней — это мысли героя, без имени)
        feed(&mut t, REAL_HW, "KiriKiriZ", 0x20, &REAL_NAMES, 9.9, 3.0);
        feed(&mut t, REAL_HW, "KiriKiriZ", 0x12, &REAL_UI, 1.0, 4.7);
        t
    }

    #[test]
    fn real_report_dialog_wins_over_menu_text() {
        let t = real_tracker();
        let ranked = t.ranked();
        assert_eq!(ranked[0].hookname, "TextRender");
        assert!(is_confident(&ranked));
    }

    #[test]
    fn name_thread_is_found() {
        let t = real_tracker();
        let dialog_key = (REAL_CODE.to_string(), 0, 0);
        assert_eq!(find_name_thread(&t, &dialog_key), Some((REAL_HW.to_string(), 0x20, 0)));
    }

    #[test]
    fn name_thread_not_invented() {
        let mut t = ThreadTracker::default();
        feed(&mut t, "D", "TextRender", 0, &REAL_DIALOG, 10.0, 3.0);
        feed(&mut t, "UI", "KiriKiriZ", 0x12, &REAL_UI, 1.0, 4.7);
        assert_eq!(find_name_thread(&t, &("D".to_string(), 0, 0)), None);
    }

    #[test]
    fn constant_line_is_not_a_name_thread() {
        let mut t = ThreadTracker::default();
        feed(&mut t, "D", "TextRender", 0, &REAL_DIALOG, 10.0, 3.0);
        feed(&mut t, "TITLE", "TextOutW", 0x50, &["Senren*Banka"; 7], 10.0, 3.0);
        assert_eq!(find_name_thread(&t, &("D".to_string(), 0, 0)), None);
    }
}
