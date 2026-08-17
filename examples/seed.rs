//! Fills a scratch history db with plausible data so the UI can be reviewed
//! without dictating for a month. Writes to $VOICEFLOW_SEED_DB.
//!
//!   VOICEFLOW_SEED_DB=/tmp/h.sqlite3 cargo run --example seed

use anyhow::Result;
use chrono::{Duration, Local};
use rusqlite::{Connection, params};

const SAMPLES: [&str; 12] = [
    "надо переписать обработчик очереди, он падает на пустом сообщении",
    "созвон в четверг перенесли на пятницу, предупреди команду",
    "купить молоко, хлеб и что-нибудь к чаю",
    "в проде опять выросла задержка на ручке поиска, посмотри трейсы",
    "напиши в ответ что мы согласны на эти условия но нужен месяц на интеграцию",
    "идея: сделать отдельный экран со статистикой диктовок и графиком по дням",
    "проверить бэкапы базы за прошлую неделю",
    "тут нужен индекс по полю created_at иначе выборка едет по всей таблице",
    "напомни завтра утром позвонить в сервис по поводу ноутбука",
    "давай вынесем этот кусок в отдельный модуль, он разросся до трёхсот строк",
    "закрыть задачу и написать в тред что всё выкачено",
    "мысль на подумать: кэшировать ответы модели по хешу входа, экономия будет заметная",
];

fn main() -> Result<()> {
    let path = std::env::var("VOICEFLOW_SEED_DB")?;
    let _ = std::fs::remove_file(&path);
    let conn = Connection::open(&path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS transcriptions (
            id          INTEGER PRIMARY KEY,
            ts          INTEGER NOT NULL,
            text        TEXT    NOT NULL,
            lang        TEXT    NOT NULL DEFAULT '',
            duration_ms INTEGER NOT NULL DEFAULT 0,
            words       INTEGER NOT NULL DEFAULT 0,
            chars       INTEGER NOT NULL DEFAULT 0
         );",
    )?;

    let mut rng: u64 = 0x5eed_1234;
    let mut next = || {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (rng >> 33) as usize
    };

    let mut rows = 0;
    for day in (0..29).rev() {
        // A couple of gaps so the streak logic has something to chew on.
        if matches!(day, 12 | 13 | 21) {
            continue;
        }
        let per_day = 1 + next() % 6;
        for _ in 0..per_day {
            let text = SAMPLES[next() % SAMPLES.len()];
            let hour = 9 + next() % 12;
            let ts = (Local::now() - Duration::days(day)).date_naive()
                .and_hms_opt(hour as u32, (next() % 60) as u32, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap()
                .timestamp();
            let words = text.split_whitespace().count() as i64;
            // Roughly 150 spoken words per minute.
            let duration_ms = (words as f64 / 150.0 * 60_000.0) as i64 + 400;
            conn.execute(
                "INSERT INTO transcriptions (ts, text, lang, duration_ms, words, chars)
                 VALUES (?1, ?2, 'ru-RU', ?3, ?4, ?5)",
                params![ts, text, duration_ms, words, text.chars().count() as i64],
            )?;
            rows += 1;
        }
    }

    println!("seeded {rows} rows into {path}");
    Ok(())
}
