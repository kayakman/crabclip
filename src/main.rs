use cliprust::db::Db;
use cliprust::settings::Settings;

fn usage() {
    println!(
        "cliprust {} — forever clipboard history\n\n\
         Usage:\n  cliprust                         run the menu bar app\n  \
         cliprust --import-collective [archive...]   import Collective.app history (default: its container)\n  \
         cliprust --stats                 print database stats\n  \
         cliprust --search <query>        search from the terminal",
        env!("CARGO_PKG_VERSION")
    );
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        cliprust::log!("PANIC: {info}");
    }));
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => cliprust::ui::run(),
        Some(a) if a.starts_with("-psn") || a == "--takeover" => cliprust::ui::run(),
        Some("--import-collective") => {
            let db = Db::open(&Db::default_path()).expect("open db");
            let settings = Settings::load(&db);
            let paths: Vec<std::path::PathBuf> = if args.len() > 1 {
                args[1..].iter().map(Into::into).collect()
            } else {
                cliprust::collective::default_paths()
            };
            if paths.is_empty() {
                println!("No Collective archive found.");
                std::process::exit(1);
            }
            let mut summary = Vec::new();
            for p in paths {
                let mut last = std::time::Instant::now();
                let r = cliprust::collective::import(&p, &db, &settings, |st| {
                    if last.elapsed().as_secs() >= 2 {
                        last = std::time::Instant::now();
                        eprintln!("  {}/{} added {} merged {}", st.added + st.merged + st.skipped, st.total, st.added, st.merged);
                    }
                });
                match r {
                    Ok(st) => summary.push(format!(
                        "{}: {} clips, {} added, {} already there, {} skipped",
                        p.file_name().unwrap_or_default().to_string_lossy(),
                        st.total,
                        st.added,
                        st.merged,
                        st.skipped
                    )),
                    Err(e) => summary.push(format!("{}: {e}", p.display())),
                }
            }
            let _ = db.vacuum_some();
            println!("{}", summary.join("\n"));
        }
        Some("--stats") => {
            let db = Db::open(&Db::default_path()).expect("open db");
            println!(
                "{} clips, {} on disk ({})",
                db.count().unwrap_or(0),
                cliprust::timefmt::bytes(db.file_size()),
                Db::default_path().display()
            );
        }
        Some("--search") => {
            let db = Db::open(&Db::default_path()).expect("open db");
            let q = args[1..].join(" ");
            let t0 = std::time::Instant::now();
            let ids = db
                .search(&cliprust::db::Filter { query: &q, starred: false, app: None }, 50)
                .unwrap_or_default();
            let dt = t0.elapsed();
            for id in &ids {
                if let Ok(Some(r)) = db.row(*id) {
                    let line: String = r.preview.lines().next().unwrap_or("").chars().take(90).collect();
                    println!(
                        "{:>6} {:<20} {:<6} {}",
                        id,
                        cliprust::timefmt::human(r.copied_at),
                        r.kind.as_str(),
                        line
                    );
                }
            }
            eprintln!("{} results in {:?}", ids.len(), dt);
        }
        _ => usage(),
    }
}
