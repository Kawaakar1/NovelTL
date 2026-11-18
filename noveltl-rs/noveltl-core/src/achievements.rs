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
    pub have: u64,
    pub need: u64,
}

impl Badge {
    pub fn earned(&self) -> bool {
        self.have >= self.need
    }
}

pub fn badges(t: Totals) -> Vec<Badge> {
    let b = |name, have, need| Badge { name, have, need };
    vec![
        b("Первая игра", t.games, 1),
        b("100 реплик", t.lines, 100),
        b("1 000 реплик", t.lines, 1_000),
        b("10 000 реплик", t.lines, 10_000),
        b("7 дней подряд", t.best_streak, 7),
        b("30 дней подряд", t.best_streak, 30),
        b("100 слов в Anki", t.anki, 100),
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
    }
}
