//! Importer for Clipy (github.com/Clipy/Clipy). Current Clipy keeps everything
//! in SQLite: `pasteboardHistories` + `pasteboardHistoryAssets` (each pasteboard
//! type byte-for-byte, in order) and `snippetFolders` + `snippets`. Older
//! Clipy (Realm) converts its data to that file when the current version
//! starts. Both versions export snippets as XML:
//! <folders><folder><title/><snippets><snippet><title/><content/>…

use crate::db::{AppInfo, Db, Inserted, Rep};
use crate::ingest::{self, Capture};
use crate::settings::Settings;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const BUNDLE: &str = "com.clipy-app.Clipy";

fn support_dirs() -> Vec<PathBuf> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    vec![
        home.join("Library/Application Support").join(BUNDLE),
        home.join("Library/Containers").join(BUNDLE).join("Data/Library/Application Support").join(BUNDLE),
    ]
}

/// Clipy's database, if it has one on this Mac.
pub fn default_db() -> Option<PathBuf> {
    support_dirs().into_iter().map(|d| d.join("sqlite.db")).find(|p| p.exists())
}

/// Data of an old Clipy (Realm) that was never opened by a current version.
pub fn legacy_realm() -> Option<PathBuf> {
    support_dirs().into_iter().map(|d| d.join("default.realm")).find(|p| p.exists())
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    pub history: usize,
    pub added: usize,
    pub merged: usize,
    pub skipped: usize,
    pub folders: usize,
    pub snippets: usize,
}

impl Stats {
    pub fn summary(&self) -> String {
        format!(
            "{} history clips ({} added, {} already there, {} skipped), {} snippets in {} folders",
            self.history, self.added, self.merged, self.skipped, self.snippets, self.folders
        )
    }
}

/// Import a Clipy database or snippets XML export into `db`.
pub fn import(path: &Path, db: &Db, settings: &Settings, progress: impl FnMut(&Stats)) -> Result<Stats, String> {
    crate::log!("clipy: importing {}", path.display());
    let head = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let st = if head.starts_with(b"SQLite format 3\0") {
        import_sqlite(path, db, settings, progress)?
    } else {
        import_xml(&String::from_utf8_lossy(&head), db, settings)?
    };
    crate::log!("clipy: {} -> {:?}", path.display(), st);
    Ok(st)
}

struct Folder {
    title: String,
    index: i64,
    snippets: Vec<(String, String, i64)>, // title, content, index
}

fn import_sqlite(path: &Path, db: &Db, settings: &Settings, mut progress: impl FnMut(&Stats)) -> Result<Stats, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let src = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    src.busy_timeout(std::time::Duration::from_secs(10)).map_err(e)?;
    let has = |table: &str| -> bool {
        src.query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", [table], |_| Ok(()))
            .optional()
            .ok()
            .flatten()
            .is_some()
    };
    if !has("pasteboardHistories") && !has("snippetFolders") {
        return Err("not a Clipy database".into());
    }
    let mut st = Stats::default();

    if has("pasteboardHistories") && has("pasteboardHistoryAssets") {
        let created = src
            .prepare("SELECT 1 FROM pragma_table_info('pasteboardHistories') WHERE name='createdAt'")
            .and_then(|mut s| s.exists([]))
            .unwrap_or(false);
        let sql = format!(
            "SELECT id, updateAt FROM pasteboardHistories ORDER BY {} ",
            if created { "createdAt, updateAt" } else { "updateAt" }
        );
        let rows: Vec<(String, i64)> = {
            let mut q = src.prepare(&sql).map_err(e)?;
            q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?.collect::<rusqlite::Result<_>>().map_err(e)?
        };
        st.history = rows.len();
        let mut assets = src
            .prepare(r#"SELECT pasteboardType, data FROM pasteboardHistoryAssets WHERE pasteboardHistoryID=? ORDER BY "index""#)
            .map_err(e)?;
        for (id, when) in rows {
            // Clipy doesn't keep item boundaries: the nth asset of a type
            // belongs to the nth item.
            let mut seen: HashMap<String, i64> = HashMap::new();
            let reps: Vec<Rep> = assets
                .query_map([&id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))
                .map_err(e)?
                .filter_map(|r| r.ok())
                .map(|(uti, data)| {
                    let n = seen.entry(uti.clone()).or_insert(0);
                    let rep = Rep { item: *n, uti, data };
                    *n += 1;
                    rep
                })
                .collect();
            let cap = Capture { ts: to_ms(when), reps, ..Default::default() };
            match ingest::build(cap, settings).map(|c| db.insert(&c, settings.zstd_level)) {
                Some(Ok(Inserted::New(_))) => st.added += 1,
                Some(Ok(Inserted::Existing(_))) => st.merged += 1,
                Some(Err(err)) => {
                    crate::log!("clipy: insert failed: {err}");
                    st.skipped += 1;
                }
                None => st.skipped += 1,
            }
            progress(&st);
        }
    }

    if has("snippetFolders") && has("snippets") {
        let folders: Vec<(String, String, i64)> = {
            let mut q = src.prepare(r#"SELECT id, title, "index" FROM snippetFolders ORDER BY "index""#).map_err(e)?;
            q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(e)?.collect::<rusqlite::Result<_>>().map_err(e)?
        };
        let mut q = src.prepare(r#"SELECT title, content, "index" FROM snippets WHERE folderID=? ORDER BY "index""#).map_err(e)?;
        let mut all = Vec::new();
        for (id, title, index) in folders {
            let snippets = q
                .query_map([&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(e)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(e)?;
            all.push(Folder { title, index, snippets });
        }
        add_snippets(all, db, settings, &mut st)?;
    }
    Ok(st)
}

fn import_xml(xml: &str, db: &Db, settings: &Settings) -> Result<Stats, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("not a Clipy snippets export: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "folders" {
        return Err("not a Clipy snippets export (no <folders>)".into());
    }
    let child = |n: roxmltree::Node, name: &str| -> String {
        n.children().find(|c| c.has_tag_name(name)).and_then(|c| c.text()).unwrap_or("").to_string()
    };
    let mut all = Vec::new();
    for (i, f) in root.children().filter(|n| n.has_tag_name("folder")).enumerate() {
        let snippets = f
            .children()
            .filter(|n| n.has_tag_name("snippets"))
            .flat_map(|s| s.children().filter(|n| n.has_tag_name("snippet")))
            .enumerate()
            .map(|(j, s)| (child(s, "title"), child(s, "content"), j as i64))
            .collect();
        all.push(Folder { title: child(f, "title"), index: i as i64, snippets });
    }
    let mut st = Stats::default();
    add_snippets(all, db, settings, &mut st)?;
    Ok(st)
}

/// Snippets become text clips filed in top-level folders named like Clipy's,
/// in Clipy's order. They are dated just before the oldest clip so they don't
/// crowd the top of the history (a snippet already in it keeps its date).
fn add_snippets(folders: Vec<Folder>, db: &Db, settings: &Settings, st: &mut Stats) -> Result<(), String> {
    let e = |e: rusqlite::Error| e.to_string();
    let oldest: Option<i64> = db.conn.query_row("SELECT MIN(created_at) FROM clips", [], |r| r.get(0)).map_err(e)?;
    let ts = oldest.unwrap_or_else(crate::timefmt::now_ms) - 60_000;
    let app = AppInfo {
        bundle: Some(BUNDLE.into()),
        name: Some("Clipy".into()),
        path: Some("/Applications/Clipy.app".into()).filter(|p: &String| Path::new(p).exists()),
    };
    if !db.has_app(BUNDLE).unwrap_or(false) {
        let icon = app.path.as_deref().and_then(crate::appicon::png_for_path);
        let _ = db.save_app(BUNDLE, app.name.as_deref(), app.path.as_deref(), icon.as_deref());
    }
    for f in folders {
        let name = f.title.replace('/', "∕");
        let name = if name.trim().is_empty() { "Untitled".to_string() } else { name };
        let fid = db.make_folder(&name).map_err(e)?;
        db.set_folder_pos(fid, f.index).map_err(e)?;
        st.folders += 1;
        for (title, content, index) in f.snippets {
            let reps = vec![Rep { item: 0, uti: "public.utf8-plain-text".into(), data: content.into_bytes() }];
            let cap = Capture { ts, reps, app: app.clone(), ..Default::default() };
            let Some(clip) = ingest::build(cap, settings) else {
                st.skipped += 1;
                continue;
            };
            let id = db.insert(&clip, settings.zstd_level).map_err(e)?.id();
            db.file_snippet(fid, id, title.trim(), Some(index)).map_err(e)?;
            st.snippets += 1;
        }
    }
    Ok(())
}

/// Clipy stores unix seconds; take milliseconds too, should a version use them.
fn to_ms(t: i64) -> i64 {
    if t > 100_000_000_000 { t } else { t * 1000 }
}
