use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use rusqlite::{Connection, params};

/// Words per minute assumed for the "time saved" figure. FluidVoice makes this
/// user-configurable; 40 is a common average for sustained prose typing.
pub const TYPING_WPM: f64 = 40.0;

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: i64,
    pub ts: i64,
    pub text: String,
    pub lang: String,
    pub duration_ms: i64,
    pub words: i64,
    pub chars: i64,
}

impl Entry {
    pub fn local(&self) -> DateTime<Local> {
        Local.timestamp_opt(self.ts, 0).single().unwrap_or_default()
    }
}

pub fn db_path() -> String {
    std::env::var("VOICEFLOW_DB")
        .unwrap_or_else(|_| format!("{}/history.sqlite3", crate::data_dir()))
}

pub fn open() -> Result<Connection> {
    std::fs::create_dir_all(crate::data_dir()).ok();
    let conn = Connection::open(db_path()).context("opening history db")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS transcriptions (
            id          INTEGER PRIMARY KEY,
            ts          INTEGER NOT NULL,
            text        TEXT    NOT NULL,
            lang        TEXT    NOT NULL DEFAULT '',
            duration_ms INTEGER NOT NULL DEFAULT 0,
            words       INTEGER NOT NULL DEFAULT 0,
            chars       INTEGER NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS idx_transcriptions_ts ON transcriptions(ts DESC);",
    )?;
    Ok(())
}

pub fn count_words(text: &str) -> i64 {
    text.split_whitespace().count() as i64
}

/// Empty transcriptions are dropped rather than stored, matching FluidVoice.
pub fn insert(conn: &Connection, text: &str, lang: &str, duration_ms: i64) -> Result<Option<i64>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    conn.execute(
        "INSERT INTO transcriptions (ts, text, lang, duration_ms, words, chars)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            Local::now().timestamp(),
            text,
            lang,
            duration_ms,
            count_words(text),
            text.chars().count() as i64,
        ],
    )?;
    Ok(Some(conn.last_insert_rowid()))
}

/// `%` and `_` typed into the search box are literal characters, not wildcards.
fn like_pattern(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 2);
    out.push('%');
    for c in query.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

pub fn list(conn: &Connection, query: &str, limit: usize) -> Result<Vec<Entry>> {
    let like = like_pattern(query.trim());
    let mut stmt = conn.prepare(
        "SELECT id, ts, text, lang, duration_ms, words, chars
           FROM transcriptions
          WHERE ?1 = '' OR text LIKE ?2 ESCAPE '\\'
       ORDER BY ts DESC, id DESC
          LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![query.trim(), like, limit as i64], |r| {
        Ok(Entry {
            id: r.get(0)?,
            ts: r.get(1)?,
            text: r.get(2)?,
            lang: r.get(3)?,
            duration_ms: r.get(4)?,
            words: r.get(5)?,
            chars: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM transcriptions WHERE id = ?1", params![id])?;
    Ok(())
}

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub total: i64,
    pub total_words: i64,
    pub words_today: i64,
    pub sessions_today: i64,
    pub speaking_secs: f64,
    pub avg_words: f64,
    pub streak: i64,
    pub best_streak: i64,
    pub peak_hour: Option<i64>,
    pub longest_words: i64,
    pub most_words_day: i64,
    pub most_sessions_day: i64,
}

impl Stats {
    /// Typing the same words at `TYPING_WPM` minus the time actually spent
    /// speaking. Negative would mean dictation was slower, so it is floored.
    pub fn time_saved_secs(&self) -> f64 {
        let typing = self.total_words as f64 / TYPING_WPM * 60.0;
        (typing - self.speaking_secs).max(0.0)
    }
}

pub fn stats(conn: &Connection) -> Result<Stats> {
    let mut s = Stats::default();

    let row = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(words),0), COALESCE(SUM(duration_ms),0),
                COALESCE(MAX(words),0)
           FROM transcriptions",
        [],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        },
    )?;
    s.total = row.0;
    s.total_words = row.1;
    s.speaking_secs = row.2 as f64 / 1000.0;
    s.longest_words = row.3;
    s.avg_words = if s.total > 0 {
        s.total_words as f64 / s.total as f64
    } else {
        0.0
    };

    let today = Local::now().date_naive();
    let (start, end) = day_bounds(today);
    let row = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(words),0) FROM transcriptions WHERE ts >= ?1 AND ts < ?2",
        params![start, end],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )?;
    s.sessions_today = row.0;
    s.words_today = row.1;

    // Both numbers must describe the SAME day: taking two independent maxima
    // reports a word count from one day next to a session count from another.
    let row = conn
        .query_row(
            "SELECT SUM(words) AS w, COUNT(*) AS c
               FROM transcriptions
              GROUP BY date(ts,'unixepoch','localtime')
              ORDER BY w DESC, c DESC LIMIT 1",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .unwrap_or((0, 0));
    s.most_words_day = row.0;
    s.most_sessions_day = row.1;

    s.peak_hour = conn
        .query_row(
            "SELECT CAST(strftime('%H', ts, 'unixepoch', 'localtime') AS INTEGER) AS h
               FROM transcriptions
              GROUP BY h ORDER BY COUNT(*) DESC, h ASC LIMIT 1",
            [],
            |r| r.get::<_, i64>(0),
        )
        .ok();

    let days = active_days(conn)?;
    (s.streak, s.best_streak) = streaks(&days, today);
    Ok(s)
}

fn day_bounds(d: NaiveDate) -> (i64, i64) {
    let start = d.and_hms_opt(0, 0, 0).unwrap();
    let end = start + chrono::Duration::days(1);
    (
        Local.from_local_datetime(&start).unwrap().timestamp(),
        Local.from_local_datetime(&end).unwrap().timestamp(),
    )
}

fn active_days(conn: &Connection) -> Result<Vec<NaiveDate>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT date(ts,'unixepoch','localtime') FROM transcriptions ORDER BY 1",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = vec![];
    for r in rows {
        if let Ok(d) = NaiveDate::parse_from_str(&r?, "%Y-%m-%d") {
            out.push(d);
        }
    }
    Ok(out)
}

/// Current streak counts back from today (a gap of exactly one day still counts
/// if nothing was dictated yet today).
fn streaks(days: &[NaiveDate], today: NaiveDate) -> (i64, i64) {
    if days.is_empty() {
        return (0, 0);
    }

    let mut best = 1i64;
    let mut run = 1i64;
    for w in days.windows(2) {
        if (w[1] - w[0]).num_days() == 1 {
            run += 1;
            best = best.max(run);
        } else {
            run = 1;
        }
    }

    let last = *days.last().unwrap();
    let gap = (today - last).num_days();
    let current = if gap > 1 {
        0
    } else {
        let mut n = 1i64;
        for w in days.windows(2).rev() {
            if (w[1] - w[0]).num_days() == 1 {
                n += 1;
            } else {
                break;
            }
        }
        n
    };
    (current, best)
}

/// Word totals per day for the last `days` days, oldest first.
pub fn daily_words(conn: &Connection, days: i64) -> Result<Vec<(NaiveDate, i64)>> {
    let today = Local::now().date_naive();
    let first = today - chrono::Duration::days(days - 1);
    let (start, _) = day_bounds(first);

    let mut stmt = conn.prepare(
        "SELECT date(ts,'unixepoch','localtime') AS d, SUM(words)
           FROM transcriptions WHERE ts >= ?1 GROUP BY d",
    )?;
    let rows = stmt.query_map(params![start], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;

    let mut map = std::collections::HashMap::new();
    for r in rows {
        let (d, w) = r?;
        if let Ok(d) = NaiveDate::parse_from_str(&d, "%Y-%m-%d") {
            map.insert(d, w);
        }
    }

    Ok((0..days)
        .map(|i| {
            let d = first + chrono::Duration::days(i);
            (d, map.get(&d).copied().unwrap_or(0))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        migrate(&c).unwrap();
        c
    }

    fn add(c: &Connection, ts: i64, text: &str) {
        c.execute(
            "INSERT INTO transcriptions (ts, text, lang, duration_ms, words, chars)
             VALUES (?1, ?2, 'ru-RU', 1000, ?3, ?4)",
            params![ts, text, count_words(text), text.chars().count() as i64],
        )
        .unwrap();
    }

    fn ts_days_ago(n: i64) -> i64 {
        let d = Local::now().date_naive() - chrono::Duration::days(n);
        day_bounds(d).0 + 3600 * 12
    }

    #[test]
    fn empty_text_is_not_stored() {
        let c = mem();
        assert!(insert(&c, "   ", "ru-RU", 0).unwrap().is_none());
        assert_eq!(stats(&c).unwrap().total, 0);
    }

    #[test]
    fn counts_words_and_today() {
        let c = mem();
        add(&c, ts_days_ago(0), "раз два три");
        add(&c, ts_days_ago(1), "вчера было четыре слова");
        let s = stats(&c).unwrap();
        assert_eq!(s.total, 2);
        assert_eq!(s.total_words, 7);
        assert_eq!(s.words_today, 3);
        assert_eq!(s.sessions_today, 1);
        assert_eq!(s.longest_words, 4);
    }

    #[test]
    fn streak_counts_consecutive_days() {
        let c = mem();
        for d in [0, 1, 2, 5, 6] {
            add(&c, ts_days_ago(d), "слово");
        }
        let s = stats(&c).unwrap();
        assert_eq!(s.streak, 3, "today plus the two days before it");
        assert_eq!(s.best_streak, 3);
    }

    #[test]
    fn streak_breaks_after_a_gap() {
        let c = mem();
        for d in [4, 5, 6] {
            add(&c, ts_days_ago(d), "слово");
        }
        assert_eq!(stats(&c).unwrap().streak, 0);
        assert_eq!(stats(&c).unwrap().best_streak, 3);
    }

    #[test]
    fn yesterday_still_counts_as_a_live_streak() {
        let c = mem();
        for d in [1, 2] {
            add(&c, ts_days_ago(d), "слово");
        }
        assert_eq!(stats(&c).unwrap().streak, 2);
    }

    #[test]
    fn daily_words_pads_missing_days() {
        let c = mem();
        add(&c, ts_days_ago(0), "раз два");
        add(&c, ts_days_ago(2), "три");
        let d = daily_words(&c, 3).unwrap();
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].1, 1);
        assert_eq!(d[1].1, 0);
        assert_eq!(d[2].1, 2);
    }

    #[test]
    fn search_filters_by_text() {
        let c = mem();
        add(&c, ts_days_ago(0), "привет мир");
        add(&c, ts_days_ago(0), "пока мир");
        assert_eq!(list(&c, "привет", 50).unwrap().len(), 1);
        assert_eq!(list(&c, "", 50).unwrap().len(), 2);
    }

    #[test]
    fn wildcards_in_the_query_are_literal() {
        let c = mem();
        add(&c, ts_days_ago(0), "скидка 100% на всё");
        add(&c, ts_days_ago(0), "обычная запись без знаков");
        // A bare `%` used to match every row.
        assert_eq!(list(&c, "%", 50).unwrap().len(), 1);
        assert_eq!(list(&c, "100%", 50).unwrap().len(), 1);
        assert_eq!(list(&c, "_", 50).unwrap().len(), 0);
    }

    #[test]
    fn best_day_reports_one_single_day() {
        let c = mem();
        // Day A: many words in one go. Day B: many sessions, few words.
        add(&c, ts_days_ago(1), "раз два три четыре пять шесть семь восемь");
        for _ in 0..5 {
            add(&c, ts_days_ago(2), "коротко");
        }
        let s = stats(&c).unwrap();
        assert_eq!(s.most_words_day, 8);
        assert_eq!(
            s.most_sessions_day, 1,
            "session count must come from the same day as the word count"
        );
    }

    #[test]
    fn time_saved_subtracts_speaking_time() {
        let s = Stats {
            total_words: 400,
            speaking_secs: 120.0,
            ..Default::default()
        };
        // 400 words at 40 wpm = 600 s of typing, minus 120 s spoken.
        assert_eq!(s.time_saved_secs(), 480.0);
    }

    #[test]
    fn time_saved_never_goes_negative() {
        let s = Stats {
            total_words: 10,
            speaking_secs: 600.0,
            ..Default::default()
        };
        assert_eq!(s.time_saved_secs(), 0.0);
    }
}
