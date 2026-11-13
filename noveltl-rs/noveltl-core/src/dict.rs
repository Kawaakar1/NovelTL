//! Словарь JMdict в SQLite: импорт, поиск слова в тексте, разбивка на сегменты с фуриганой.

use rusqlite::{params, Connection, OpenFlags};
use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::deinflect::deinflect;

pub const JMDICT_URL: &str = "https://www.edrdg.org/pub/Nihongo/JMdict.gz";
pub const ATTRIBUTION: &str = "JMdict © Electronic Dictionary Research and Development Group, CC BY-SA 4.0";

#[derive(Clone, Debug, PartialEq)]
pub struct Entry { pub forms: Vec<String>, pub readings: Vec<String>, pub glosses: Vec<String> }

#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    /// длина найденного слова в тексте, в символах (char)
    pub len: usize,
    /// словарная форма, по которой нашлось
    pub form: String,
    pub entry: Entry,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub text: String,
    /// чтение для фуриганы — только над иероглифами слова; None — фуригана не нужна
    pub reading: Option<String>,
    /// номер первого символа (char) сегмента в исходном тексте
    pub start: usize,
}

pub struct Dictionary { conn: Connection }

const SEP: &str = "\n"; // разделитель значений внутри колонки
const MAX_WORD: usize = 12;

fn e2s(e: impl std::fmt::Display) -> String { e.to_string() }

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Содержимое между `>` первого тега и последним `</`.
fn inner(line: &str) -> &str {
    let s = line.find('>').map_or(0, |i| i + 1);
    let e = line.rfind("</").filter(|&e| e >= s).unwrap_or(line.len());
    &line[s..e]
}

fn is_kanji(c: char) -> bool { matches!(c, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{3005}') }
fn is_kana(c: char) -> bool { matches!(c, '\u{3040}'..='\u{30ff}') }

#[derive(Default)]
struct Raw { forms: Vec<String>, readings: Vec<String>, en: Vec<String>, ru: Vec<String>, common: bool }

impl Dictionary {
    /// Открыть готовую базу. Err — базы нет или она повреждена.
    pub fn open(db: &Path) -> Result<Dictionary, String> {
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(e2s)?;
        conn.query_row("SELECT 1 FROM entry LIMIT 1", [], |_| Ok(())).map_err(e2s)?;
        Ok(Dictionary { conn })
    }

    /// Импорт JMdict.gz в новую базу db (старая заменяется). Возвращает число статей.
    pub fn import_gz(gz: &Path, db: &Path) -> Result<usize, String> {
        let f = std::fs::File::open(gz).map_err(e2s)?;
        Self::import_xml(std::io::BufReader::new(flate2::read::GzDecoder::new(f)), db)
    }

    /// Импорт из распакованного XML-потока: пишем во временный файл, потом переименовываем.
    pub fn import_xml(xml: impl BufRead, db: &Path) -> Result<usize, String> {
        let mut tmp: std::ffi::OsString = db.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let _ = std::fs::remove_file(&tmp);
        let mut bak: std::ffi::OsString = db.as_os_str().to_owned();
        bak.push(".bak");
        let bak = PathBuf::from(bak);
        let res = Self::build(xml, &tmp).and_then(|n| {
            // старая база уходит в .bak и возвращается, если новая не встала на место
            match std::fs::remove_file(&bak) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e2s(e)),
                _ => {}
            }
            let had_old = db.exists();
            if had_old { std::fs::rename(db, &bak).map_err(e2s)?; }
            if let Err(e) = std::fs::rename(&tmp, db) {
                if had_old { std::fs::rename(&bak, db).map_err(|e2| format!("{e}; {e2}"))?; }
                return Err(e2s(e));
            }
            let _ = std::fs::remove_file(&bak); // не критично: следующий импорт уберёт остаток
            Ok(n)
        });
        if res.is_err() { let _ = std::fs::remove_file(&tmp); }
        res
    }

    fn build(xml: impl BufRead, path: &Path) -> Result<usize, String> {
        let mut conn = Connection::open(path).map_err(e2s)?;
        conn.execute_batch(
            "PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;
             CREATE TABLE entry(id INTEGER PRIMARY KEY, forms TEXT, readings TEXT, en TEXT, ru TEXT, common INTEGER);
             CREATE TABLE form(form TEXT, entry INTEGER);",
        ).map_err(e2s)?;
        let tx = conn.transaction().map_err(e2s)?;
        let mut count = 0usize;
        {
            let mut ins_e = tx.prepare("INSERT INTO entry VALUES(?,?,?,?,?,?)").map_err(e2s)?;
            let mut ins_f = tx.prepare("INSERT INTO form VALUES(?,?)").map_err(e2s)?;
            let mut cur = Raw::default();
            let (mut s_en, mut s_ru): (Option<String>, Option<String>) = (None, None);
            for line in xml.lines() {
                let line = line.map_err(e2s)?;
                let l = line.trim();
                if l.starts_with("<entry>") { cur = Raw::default(); }
                else if l.starts_with("<keb>") { cur.forms.push(unescape(inner(l))); }
                else if l.starts_with("<reb>") { cur.readings.push(unescape(inner(l))); }
                else if l.starts_with("<ke_pri>") || l.starts_with("<re_pri>") { cur.common = true; }
                else if l.starts_with("<sense>") { s_en = None; s_ru = None; }
                else if l.starts_with("<gloss") {
                    let tag = &l[..l.find('>').unwrap_or(l.len())];
                    let slot = if tag.contains("xml:lang=\"rus\"") { &mut s_ru }
                               else if tag.contains("xml:lang=") && !tag.contains("xml:lang=\"eng\"") { continue }
                               else { &mut s_en };
                    if slot.is_none() {
                        let g = unescape(inner(l));
                        // в русских глоссах бывает нумерация «1) …»
                        let g = g.split_once(") ").filter(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).map_or(g.as_str(), |(_, r)| r);
                        *slot = Some(g.to_string());
                    }
                }
                else if l.starts_with("</sense>") {
                    if let Some(g) = s_en.take() { cur.en.push(g); }
                    if let Some(g) = s_ru.take() { cur.ru.push(g); }
                }
                else if l.starts_with("</entry>") {
                    if cur.forms.is_empty() && cur.readings.is_empty() { continue; }
                    count += 1;
                    let id = count as i64;
                    let top = |v: &[String]| v.iter().take(5).cloned().collect::<Vec<_>>().join(SEP);
                    ins_e.execute(params![id, cur.forms.join(SEP), cur.readings.join(SEP), top(&cur.en), top(&cur.ru), cur.common])
                        .map_err(e2s)?;
                    for f in cur.forms.iter().chain(&cur.readings) { ins_f.execute(params![f, id]).map_err(e2s)?; }
                }
            }
        }
        tx.execute_batch("CREATE INDEX form_idx ON form(form);").map_err(e2s)?;
        tx.commit().map_err(e2s)?;
        Ok(count)
    }

    /// Статьи по набору форм одним запросом: для каждой формы — лучшая (common, затем id).
    fn find_many(&self, forms: &[String], prefer_ru: bool) -> std::collections::HashMap<String, Entry> {
        let mut found = std::collections::HashMap::new();
        if forms.is_empty() { return found; }
        let marks = vec!["?"; forms.len()].join(",");
        let sql = format!(
            "SELECT f.form, e.forms, e.readings, e.en, e.ru FROM form f JOIN entry e ON e.id = f.entry
             WHERE f.form IN ({marks}) ORDER BY e.common DESC, e.id");
        let Ok(mut st) = self.conn.prepare(&sql) else { return found };
        let Ok(rows) = st.query_map(rusqlite::params_from_iter(forms), |r| {
            let sp = |s: String| s.split(SEP).filter(|x| !x.is_empty()).map(String::from).collect::<Vec<_>>();
            let (en, ru) = (sp(r.get(3)?), sp(r.get(4)?));
            let glosses = if (prefer_ru && !ru.is_empty()) || en.is_empty() { ru } else { en };
            Ok((r.get::<_, String>(0)?, Entry { forms: sp(r.get(1)?), readings: sp(r.get(2)?), glosses }))
        }) else { return found };
        for (form, entry) in rows.flatten() { found.entry(form).or_insert(entry); }
        found
    }

    #[cfg(test)] // эталон для теста эквивалентности
    fn find(&self, form: &str, prefer_ru: bool) -> Option<Entry> {
        let mut st = self.conn.prepare_cached(
            "SELECT e.forms, e.readings, e.en, e.ru FROM form f JOIN entry e ON e.id = f.entry
             WHERE f.form = ? ORDER BY e.common DESC, e.id LIMIT 1").ok()?;
        st.query_row([form], |r| {
            let sp = |s: String| s.split(SEP).filter(|x| !x.is_empty()).map(String::from).collect::<Vec<_>>();
            let (en, ru) = (sp(r.get(2)?), sp(r.get(3)?));
            let glosses = if (prefer_ru && !ru.is_empty()) || en.is_empty() { ru } else { en };
            Ok(Entry { forms: sp(r.get(0)?), readings: sp(r.get(1)?), glosses })
        }).ok()
    }

    /// Слово, начинающееся с символа `pos` (char): самое длинное совпадение (до 12 символов), с учётом deinflect.
    pub fn lookup_at(&self, text: &str, pos: usize, prefer_ru: bool) -> Option<Match> {
        let chars: Vec<char> = text.chars().skip(pos).take(MAX_WORD).collect();
        // кандидаты от длинных к коротким; все формы позиции — одним запросом
        let cands: Vec<(usize, String)> = (1..=chars.len()).rev()
            .flat_map(|len| { let word: String = chars[..len].iter().collect(); deinflect(&word).into_iter().map(move |f| (len, f)) })
            .collect();
        let mut forms: Vec<String> = cands.iter().map(|(_, f)| f.clone()).collect();
        forms.sort();
        forms.dedup();
        let found = self.find_many(&forms, prefer_ru);
        cands.into_iter().find_map(|(len, form)| found.get(&form).cloned().map(|entry| Match { len, form, entry }))
    }

    /// Разбивка текста на сегменты для фуриганы (жадно слева направо); несловарные символы склеиваются.
    pub fn segments(&self, text: &str) -> Vec<Segment> {
        let chars: Vec<char> = text.chars().collect();
        let mut out: Vec<Segment> = Vec::new();
        let mut plain: Option<Segment> = None; // накапливаемый несловарный хвост
        let mut i = 0;
        while i < chars.len() {
            let Some(m) = self.lookup_at(text, i, false) else {
                let c = chars[i];
                match plain.as_mut() {
                    Some(p) => p.text.push(c),
                    None => plain = Some(Segment { text: c.to_string(), reading: None, start: i }),
                }
                i += 1;
                continue;
            };
            out.extend(plain.take());
            word_segments(&chars[i..i + m.len], i, &m, &mut out);
            i += m.len;
        }
        out.extend(plain);
        out
    }
}

/// Сегменты слова: иероглифы с чтением (без окуригана) + остаток без чтения.
fn word_segments(surface: &[char], start: usize, m: &Match, out: &mut Vec<Segment>) {
    let text: String = surface.iter().collect();
    let Some(reading) = m.entry.readings.first().filter(|_| surface.iter().any(|&c| is_kanji(c))) else {
        out.push(Segment { text, reading: None, start });
        return;
    };
    let form: Vec<char> = m.form.chars().collect();
    let tail_len = form.iter().rev().take_while(|&&c| is_kana(c)).count();
    let rd: Vec<char> = reading.chars().collect();
    let lead = form.len() - tail_len;
    if tail_len == 0 || lead == 0 || lead >= surface.len() || rd.len() <= tail_len || rd[rd.len() - tail_len..] != form[lead..] {
        // окуриганы нет (или чтение с ней не сходится) — чтение над всем словом
        out.push(Segment { text, reading: Some(reading.clone()), start });
        return;
    }
    out.push(Segment { text: surface[..lead].iter().collect(), reading: Some(rd[..rd.len() - tail_len].iter().collect()), start });
    out.push(Segment { text: surface[lead..].iter().collect(), reading: None, start: start + lead });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE JMdict [
<!ENTITY n "noun (common) (futsuumeishi)">
]>
<JMdict>
<entry>
<ent_seq>1</ent_seq>
<k_ele>
<keb>食べる</keb>
<ke_pri>ichi1</ke_pri>
</k_ele>
<r_ele>
<reb>たべる</reb>
<re_pri>ichi1</re_pri>
</r_ele>
<sense>
<pos>&v1;</pos>
<gloss>to eat</gloss>
<gloss xml:lang="rus">есть</gloss>
</sense>
</entry>
<entry>
<k_ele>
<keb>猫</keb>
</k_ele>
<r_ele>
<reb>ねこ</reb>
</r_ele>
<sense>
<gloss>cat &amp; kitten</gloss>
</sense>
</entry>
<entry>
<k_ele>
<keb>今日</keb>
</k_ele>
<r_ele>
<reb>きょう</reb>
</r_ele>
<sense>
<gloss>today</gloss>
</sense>
</entry>
<entry>
<k_ele>
<keb>早い</keb>
</k_ele>
<r_ele>
<reb>はやい</reb>
</r_ele>
<sense>
<gloss>early</gloss>
</sense>
</entry>
<entry>
<k_ele>
<keb>行く</keb>
</k_ele>
<r_ele>
<reb>いく</reb>
</r_ele>
<sense>
<gloss>to go</gloss>
</sense>
</entry>
</JMdict>
"#;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("noveltl_dict_{}_{name}", std::process::id()))
    }

    fn setup(name: &str) -> (Dictionary, PathBuf) {
        let db = tmp(name);
        assert_eq!(Dictionary::import_xml(XML.as_bytes(), &db).unwrap(), 5);
        (Dictionary::open(&db).unwrap(), db)
    }

    #[test]
    fn lookup() {
        let (d, db) = setup("lookup.db");
        let m = d.lookup_at("猫が食べました", 2, false).unwrap();
        assert_eq!((m.len, m.form.as_str()), (5, "食べる"));
        assert!(m.entry.glosses.contains(&"to eat".to_string()));
        assert_eq!(d.lookup_at("猫が食べました", 2, true).unwrap().entry.glosses, vec!["есть"]);
        assert_eq!(d.lookup_at("猫", 0, false).unwrap().entry.glosses, vec!["cat & kitten"]);
        assert_eq!(d.lookup_at("今日は", 0, false).unwrap().len, 2);
        drop(d);
        let _ = std::fs::remove_file(db);
    }

    /// Старый алгоритм (по запросу на каждую форму) должен давать те же результаты.
    #[test]
    fn lookup_matches_per_form_search() {
        let (d, db) = setup("equiv.db");
        let naive = |text: &str, pos: usize, ru: bool| {
            let chars: Vec<char> = text.chars().skip(pos).take(MAX_WORD).collect();
            for len in (1..=chars.len()).rev() {
                let word: String = chars[..len].iter().collect();
                for form in deinflect(&word) {
                    if let Some(entry) = d.find(&form, ru) { return Some(Match { len, form, entry }); }
                }
            }
            None
        };
        for text in ["猫が食べました。今日は早くて行った", "食べさせられなかった", "あいうえお猫"] {
            for pos in 0..text.chars().count() {
                for ru in [false, true] {
                    assert_eq!(d.lookup_at(text, pos, ru), naive(text, pos, ru), "{text} {pos}");
                }
            }
        }
        drop(d);
        let _ = std::fs::remove_file(db);
    }

    #[test]
    fn failed_swap_keeps_old_db() {
        let db = tmp("swap.db");
        assert_eq!(Dictionary::import_xml("<entry>
<reb>あ</reb>
</entry>
".as_bytes(), &db).unwrap(), 1);
        // .bak занят каталогом: remove_file падает, ошибка не глушится, старая база цела
        let mut bak = db.as_os_str().to_owned();
        bak.push(".bak");
        let bak = PathBuf::from(bak);
        std::fs::create_dir(&bak).unwrap();
        assert!(Dictionary::import_xml(XML.as_bytes(), &db).is_err());
        assert!(Dictionary::open(&db).unwrap().lookup_at("あ", 0, false).is_some());
        let _ = std::fs::remove_dir(&bak);
        let _ = std::fs::remove_file(db);
    }

    #[test]
    fn segs() {
        let (d, db) = setup("segs.db");
        let s = |t: &str, r: Option<&str>, start| Segment { text: t.into(), reading: r.map(Into::into), start };
        assert_eq!(d.segments("食べました"), vec![s("食", Some("た"), 0), s("べました", None, 1)]);
        assert_eq!(d.segments("猫"), vec![s("猫", Some("ねこ"), 0)]);
        assert_eq!(d.segments("早くて"), vec![s("早", Some("はや"), 0), s("くて", None, 1)]);
        drop(d);
        let _ = std::fs::remove_file(db);
    }

    #[test]
    fn reimport_and_gz() {
        let db = tmp("gz.db");
        assert_eq!(Dictionary::import_xml("<entry>\n<reb>あ</reb>\n</entry>\n".as_bytes(), &db).unwrap(), 1);
        let gz = tmp("jmdict.gz");
        let mut enc = flate2::write::GzEncoder::new(std::fs::File::create(&gz).unwrap(), flate2::Compression::fast());
        enc.write_all(XML.as_bytes()).unwrap();
        enc.finish().unwrap();
        assert_eq!(Dictionary::import_gz(&gz, &db).unwrap(), 5);
        let d = Dictionary::open(&db).unwrap();
        assert!(d.lookup_at("あ", 0, false).is_none());
        assert!(d.lookup_at("行く", 0, false).is_some());
        drop(d);
        let _ = std::fs::remove_file(db);
        let _ = std::fs::remove_file(gz);
    }

    /// Проверка на настоящем JMdict: JMDICT_GZ=путь cargo test real -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_jmdict() {
        let gz = PathBuf::from(std::env::var("JMDICT_GZ").unwrap());
        // JMDICT_DB — оставить базу по этому пути (для проверки окон), иначе временная и удаляется
        let keep = std::env::var("JMDICT_DB").ok().map(PathBuf::from);
        let db = keep.clone().unwrap_or_else(|| tmp("real.db"));
        let t = std::time::Instant::now();
        let n = Dictionary::import_gz(&gz, &db).unwrap();
        println!("import: {n} entries, {:?}, db {} bytes", t.elapsed(), std::fs::metadata(&db).unwrap().len());
        let d = Dictionary::open(&db).unwrap();
        println!("{:?}", d.lookup_at("食べました", 0, true));
        println!("{:?}", d.lookup_at("食べました", 0, false));
        println!("{:?}", d.segments("猫が食べさせられました。今日は早い"));
        drop(d);
        if keep.is_none() {
            let _ = std::fs::remove_file(db);
        }
    }
}
