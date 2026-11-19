use chrono::{FixedOffset, TimeZone};
use crabclip::timefmt::{human_at, ordinal};

fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<FixedOffset> {
    FixedOffset::east_opt(3600).unwrap().with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
}

#[test]
fn labels() {
    let now = at(2026, 11, 18, 19, 40); // a Wednesday
    let ms = |t: chrono::DateTime<FixedOffset>| t.timestamp_millis();
    assert_eq!(human_at(ms(at(2026, 11, 18, 13, 20)), now), "Today 13:20");
    assert_eq!(human_at(ms(at(2026, 11, 17, 3, 45)), now), "Yesterday 3:45");
    assert_eq!(human_at(ms(at(2026, 11, 16, 10, 23)), now), "Monday 10:23");
    assert_eq!(human_at(ms(at(2026, 11, 12, 10, 23)), now), "Thursday 10:23");
    assert_eq!(human_at(ms(at(2026, 11, 11, 10, 23)), now), "Wed Nov 11th 10:23");
    assert_eq!(human_at(ms(at(2026, 8, 13, 10, 23)), now), "Thu Aug 13th 10:23");
    assert_eq!(human_at(ms(at(2024, 8, 13, 10, 23)), now), "Tue Aug 13th 2024 10:23");
    assert_eq!(human_at(ms(at(2026, 11, 1, 9, 5)), now), "Sun Nov 1st 9:05");
}

#[test]
fn ordinals() {
    let got: Vec<_> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 31].iter().map(|d| ordinal(*d)).collect();
    assert_eq!(got, ["st", "nd", "rd", "th", "th", "th", "th", "st", "nd", "rd", "st"]);
}
