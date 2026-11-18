//! Human date labels for the bottom bar: "Today 13:20", "Yesterday 3:45",
//! "Monday 10:23", "Tue Aug 13th 10:23", "Tue Aug 13th 2024 10:23".

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn ordinal(day: u32) -> &'static str {
    match (day % 10, day % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    }
}

pub fn human(ts_ms: i64) -> String {
    human_at(ts_ms, Local::now())
}

pub fn human_at<Tz: TimeZone>(ts_ms: i64, now: DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let tz = now.timezone();
    let Some(t) = tz.timestamp_millis_opt(ts_ms).single() else {
        return String::new();
    };
    let time = format!("{}:{:02}", t.hour(), t.minute());
    let days = (now.date_naive() - t.date_naive()).num_days();
    match days {
        0 => format!("Today {time}"),
        1 => format!("Yesterday {time}"),
        2..=6 => format!("{} {time}", t.format("%A")),
        _ => {
            let day = format!("{} {}{}", t.format("%a %b"), t.day(), ordinal(t.day()));
            if t.year() == now.year() {
                format!("{day} {time}")
            } else {
                format!("{day} {} {time}", t.year())
            }
        }
    }
}

/// Full timestamp for the preview metadata, e.g. "Tue Aug 13th 2024 10:23:05".
pub fn full(ts_ms: i64) -> String {
    match Local.timestamp_millis_opt(ts_ms).single() {
        Some(t) => format!(
            "{} {}{} {} {}",
            t.format("%a %b"),
            t.day(),
            ordinal(t.day()),
            t.year(),
            t.format("%H:%M:%S")
        ),
        None => String::new(),
    }
}

pub fn bytes(n: i64) -> String {
    let n = n as f64;
    if n < 1024.0 {
        format!("{n} B")
    } else if n < 1024.0 * 1024.0 {
        format!("{:.1} KB", n / 1024.0)
    } else if n < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", n / 1024.0 / 1024.0)
    } else {
        format!("{:.2} GB", n / 1024.0 / 1024.0 / 1024.0)
    }
}
