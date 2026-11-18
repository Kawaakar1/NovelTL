//! Достижения: значки по счётчикам чтения и изучения.

/// Что уже набрано: числа по всем играм.
#[derive(Clone, Copy, Debug, Default)]
pub struct Totals {
    pub lines: u64,
    pub best_streak: u64,
    pub anki: u64,
    /// игр, в которых есть хотя бы одна реплика
    pub games: u64,
}

pub struct Badge {
    /// русская строка-ключ для `t()`
    pub name: &'static str,
    /// русская строка-ключ: за что даётся значок
    pub hint: &'static str,
    /// короткая метка на медальоне
    pub short: &'static str,
    /// значок за серию (его счётчик — рекорд, а не текущая серия)
    pub streak: bool,
    pub have: u64,
    pub need: u64,
}

impl Badge {
    pub fn earned(&self) -> bool {
        self.have >= self.need
    }
}

pub fn badges(t: Totals) -> Vec<Badge> {
    let b = |name, hint, short, streak, have, need| Badge { name, hint, short, streak, have, need };
    vec![
        b("Первая игра", "Прочитайте первую реплику в любой игре", "1", false, t.games, 1),
        b("100 реплик", "Прочитайте 100 реплик во всех играх", "100", false, t.lines, 100),
        b("1 000 реплик", "Прочитайте 1 000 реплик во всех играх", "1K", false, t.lines, 1_000),
        b("10 000 реплик", "Прочитайте 10 000 реплик во всех играх", "10K", false, t.lines, 10_000),
        b("7 дней подряд", "Читайте 7 дней подряд (лучшая серия)", "7", true, t.best_streak, 7),
        b("30 дней подряд", "Читайте 30 дней подряд (лучшая серия)", "30", true, t.best_streak, 30),
        b("100 слов в Anki", "Добавьте 100 слов в Anki из NovelTL", "100", false, t.anki, 100),
    ]
}

/// Ближайший незаработанный значок: с наибольшей долей выполнения (при равенстве — первый в списке).
pub fn next_badge(badges: &[Badge]) -> Option<&Badge> {
    let frac = |b: &&Badge| b.have as f64 / b.need as f64;
    badges.iter().filter(|b| !b.earned()).reduce(|a, b| if frac(&b) > frac(&a) { b } else { a })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earned_and_next() {
        let none = badges(Totals::default());
        assert!(none.iter().all(|b| !b.earned()));
        let t = Totals { lines: 1_500, best_streak: 3, anki: 10, games: 2 };
        let bs = badges(t);
        let got: Vec<_> = bs.iter().filter(|b| b.earned()).map(|b| b.name).collect();
        assert_eq!(got, ["Первая игра", "100 реплик", "1 000 реплик"]);
        // 1500/10000 = 0.15, 3/7 = 0.43, 10/100 = 0.1
        assert_eq!(next_badge(&bs).unwrap().name, "7 дней подряд");
        let all = badges(Totals { lines: 99_999, best_streak: 99, anki: 999, games: 9 });
        assert!(next_badge(&all).is_none());
        assert!(bs.iter().all(|b| !b.hint.is_empty() && !b.short.is_empty()));
    }
}
