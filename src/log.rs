//! Tiny append-only file logger (~/Library/Logs/cliprust.log), also mirrored to stderr.

use std::io::Write;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

pub fn path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::Path::new(&home).join("Library/Logs/cliprust.log")
}

pub fn write(msg: &str) {
    let line = format!("{} {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), msg);
    eprint!("{line}");
    let _g = LOCK.lock();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path()) {
        let _ = f.write_all(line.as_bytes());
    }
}

#[macro_export]
macro_rules! log {
    ($($t:tt)*) => { $crate::log::write(&format!($($t)*)) };
}
