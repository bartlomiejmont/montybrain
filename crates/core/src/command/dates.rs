//! `@termin` words: absolute (`@2026-11-20`, `@2026-11-20T14:00`) and relative
//! (`@dzis`, `@jutro`, `@pojutrze`, `@pn`…`@nd`, `@+3d`, `@+2t`). Relative ones
//! are turned into a plain date with the clock passed in; files only ever get
//! absolute dates.

use chrono::{Datelike, Duration, NaiveDate, Weekday};

use crate::model::Due;
use crate::slug::fold;

/// A due word as written, before it meets the clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DueWord {
    Absolute(Due),
    /// Today plus this many days.
    Days(i64),
    /// The next such weekday, strictly after today (on a Monday `@pn` is in
    /// a week).
    Weekday(Weekday),
}

impl DueWord {
    pub fn resolve(self, today: NaiveDate) -> Due {
        match self {
            DueWord::Absolute(d) => d,
            DueWord::Days(n) => Due::date(today + Duration::days(n)),
            DueWord::Weekday(w) => {
                let ahead = (7 + w.num_days_from_monday() as i64
                    - today.weekday().num_days_from_monday() as i64)
                    % 7;
                Due::date(today + Duration::days(if ahead == 0 { 7 } else { ahead }))
            }
        }
    }

    pub fn is_relative(self) -> bool {
        !matches!(self, DueWord::Absolute(_))
    }
}

const WEEKDAYS: &[(&str, Weekday)] = &[
    ("mon", Weekday::Mon), ("monday", Weekday::Mon),
    ("tue", Weekday::Tue), ("tuesday", Weekday::Tue),
    ("wed", Weekday::Wed), ("wednesday", Weekday::Wed),
    ("thu", Weekday::Thu), ("thursday", Weekday::Thu),
    ("fri", Weekday::Fri), ("friday", Weekday::Fri),
    ("sat", Weekday::Sat), ("saturday", Weekday::Sat),
    ("sun", Weekday::Sun), ("sunday", Weekday::Sun),
    ("pn", Weekday::Mon),
    ("pon", Weekday::Mon),
    ("poniedzialek", Weekday::Mon),
    ("wt", Weekday::Tue),
    ("wtorek", Weekday::Tue),
    ("sr", Weekday::Wed),
    ("sroda", Weekday::Wed),
    ("cz", Weekday::Thu),
    ("czw", Weekday::Thu),
    ("czwartek", Weekday::Thu),
    ("pt", Weekday::Fri),
    ("piatek", Weekday::Fri),
    ("sb", Weekday::Sat),
    ("sob", Weekday::Sat),
    ("sobota", Weekday::Sat),
    ("nd", Weekday::Sun),
    ("ndz", Weekday::Sun),
    ("niedziela", Weekday::Sun),
];

/// The longest relative offset accepted (`@+3650d`): anything larger is a typo.
const MAX_DAYS: i64 = 3650;

/// `word` is what follows `@`. `None`: not a date, so the whole `@word` stays text.
pub fn parse_due_word(word: &str) -> Option<DueWord> {
    if let Ok(d) = word.parse::<Due>() {
        return Some(DueWord::Absolute(d));
    }
    let w = fold(word);
    match w.as_str() {
        "dzis" | "dzisiaj" | "today" => return Some(DueWord::Days(0)),
        "jutro" | "tomorrow" => return Some(DueWord::Days(1)),
        "pojutrze" | "dayaftertomorrow" => return Some(DueWord::Days(2)),
        _ => {}
    }
    if let Some((_, wd)) = WEEKDAYS.iter().find(|(n, _)| *n == w) {
        return Some(DueWord::Weekday(*wd));
    }
    let rest = w.strip_prefix('+')?;
    let (num, unit) = rest.split_at(rest.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = num.parse().ok()?;
    let days = match unit {
        "d" => n,
        "t" | "tyg" | "w" | "week" | "weeks" => n.checked_mul(7)?,
        _ => return None,
    };
    (days <= MAX_DAYS).then_some(DueWord::Days(days))
}

/// Looks like an attempt at an absolute date (`@2026-…`) but is not a valid one:
/// worth a warning, since it stays in the title.
pub fn looks_like_bad_date(word: &str) -> bool {
    let b = word.as_bytes();
    b.len() >= 5
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && parse_due_word(word).is_none()
}

const MONTHS: [&str; 12] = [
    "sty", "lut", "mar", "kwi", "maj", "cze", "lip", "sie", "wrz", "paź", "lis", "gru",
];
const DAYS: [&str; 7] = ["pn", "wt", "śr", "cz", "pt", "sb", "nd"];

/// Polish short form for the reading under the field: `dziś`, `jutro`,
/// `pojutrze`, else `pt 20 lis` (with the year when it is not this year), and
/// the time if any.
pub fn describe_due(due: &Due, today: NaiveDate) -> String {
    let d = due.date;
    let diff = (d - today).num_days();
    let mut s = match diff {
        0 => "dziś".to_string(),
        1 => "jutro".to_string(),
        2 => "pojutrze".to_string(),
        -1 => "wczoraj".to_string(),
        _ => {
            let mut s = format!(
                "{} {} {}",
                DAYS[d.weekday().num_days_from_monday() as usize],
                d.day(),
                MONTHS[d.month0() as usize]
            );
            if d.year() != today.year() {
                s.push_str(&format!(" {}", d.year()));
            }
            s
        }
    };
    if let Some(t) = due.time {
        s.push_str(&format!(" {}", t.format("%H:%M")));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn relative_dates_with_an_injected_clock() {
        // 2026-11-14 is a Saturday.
        let today = d("2026-11-14");
        let r = |w: &str| parse_due_word(w).map(|x| x.resolve(today).to_string());
        assert_eq!(r("dzis").as_deref(), Some("2026-11-14"));
        assert_eq!(r("dziś").as_deref(), Some("2026-11-14"));
        assert_eq!(r("Jutro").as_deref(), Some("2026-11-15"));
        assert_eq!(r("pojutrze").as_deref(), Some("2026-11-16"));
        assert_eq!(r("pn").as_deref(), Some("2026-11-16"));
        assert_eq!(r("śr").as_deref(), Some("2026-11-18"));
        assert_eq!(r("pt").as_deref(), Some("2026-11-20"));
        // Today's weekday means next week.
        assert_eq!(r("sb").as_deref(), Some("2026-11-21"));
        assert_eq!(r("nd").as_deref(), Some("2026-11-15"));
        assert_eq!(r("+3d").as_deref(), Some("2026-11-17"));
        assert_eq!(r("+2t").as_deref(), Some("2026-11-28"));
        assert_eq!(r("+0d").as_deref(), Some("2026-11-14"));
        assert_eq!(r("2026-11-20").as_deref(), Some("2026-11-20"));
        assert_eq!(r("2026-11-20T14:00").as_deref(), Some("2026-11-20T14:00"));
        // Across a month and a year.
        let r2 = |w: &str| {
            parse_due_word(w)
                .unwrap()
                .resolve(d("2026-12-30"))
                .to_string()
        };
        assert_eq!(r2("+3d"), "2027-01-02");
        assert_eq!(r2("jutro"), "2026-12-31");
    }

    #[test]
    fn non_dates_stay_text() {
        for w in [
            "mention",
            "2x",
            "",
            "+3",
            "+d",
            "+3m",
            "2026-02-30",
            "2026-11-20T25:00",
            "jutr",
            "+99999d",
            "ania",
        ] {
            assert!(parse_due_word(w).is_none(), "{w}");
        }
        assert!(looks_like_bad_date("2026-02-30"));
        assert!(!looks_like_bad_date("mention"));
        assert!(!looks_like_bad_date("2026-02-28"));
    }

    #[test]
    fn readings() {
        let today = d("2026-11-14");
        let due = |s: &str| s.parse::<Due>().unwrap();
        assert_eq!(describe_due(&due("2026-11-14"), today), "dziś");
        assert_eq!(describe_due(&due("2026-11-15"), today), "jutro");
        assert_eq!(describe_due(&due("2026-11-20"), today), "pt 20 lis");
        assert_eq!(
            describe_due(&due("2027-01-04T09:30"), today),
            "pn 4 sty 2027 09:30"
        );
    }
}
