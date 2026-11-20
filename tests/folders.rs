use crabclip::db::{AppInfo, Db, Entry, Filter, Kind, Rep};
use crabclip::ingest::{Capture, build};
use crabclip::settings::Settings;
use rusqlite::{Connection, params};

fn insert(db: &Db, ts: i64, text: &str) -> i64 {
    let cap = Capture {
        ts,
        reps: vec![Rep { item: 0, uti: "public.utf8-plain-text".into(), data: text.as_bytes().to_vec() }],
        app: AppInfo { bundle: Some("com.test.app".into()), name: Some("App".into()), path: None },
        ..Default::default()
    };
    db.insert(&build(cap, &Settings::default()).unwrap(), 3).unwrap().id()
}

fn search(db: &Db, q: &str) -> Vec<i64> {
    db.search(&Filter { query: q, starred: false, app: None }, 100).unwrap()
}

fn labels(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .map(|e| match e {
            Entry::Folder { name, items, .. } => format!("{name}/ ({items})"),
            Entry::Snippet { title, .. } => title.clone(),
        })
        .collect()
}

#[test]
fn folders_nest_list_and_prune() {
    let db = Db::open_memory().unwrap();
    let a = insert(&db, 1, "ssh deploy@prod.example.com");
    let b = insert(&db, 2, "VPN secret thing");
    let c = insert(&db, 3, "unfiled clip");

    let got = db.set_folders(a, &[" Clients / Formosa ".into(), "clients/formosa".into(), "Servers".into()]).unwrap();
    assert_eq!(got, vec!["Clients/Formosa", "Servers"], "paths are trimmed, case-insensitive, deduplicated");
    db.set_folders(b, &["Clients/Formosa".into()]).unwrap();
    db.file_snippet(db.folder_id("Clients/Formosa").unwrap().unwrap(), b, "2. Tunnel", None).unwrap();

    assert_eq!(labels(&db.folder_entries(0, "").unwrap()), vec!["Clients/ (1)", "Servers/ (1)"]);
    let clients = db.folder_id("clients").unwrap().unwrap();
    assert_eq!(labels(&db.folder_entries(clients, "").unwrap()), vec!["Formosa/ (2)"]);
    let formosa = db.folder_id("Clients/Formosa").unwrap().unwrap();
    assert_eq!(labels(&db.folder_entries(formosa, "").unwrap()), vec!["ssh deploy@prod.example.com", "2. Tunnel"]);
    assert_eq!(labels(&db.folder_entries(formosa, "tun").unwrap()), vec!["2. Tunnel"], "filter by name, any case");
    assert_eq!(db.folder_path(formosa).unwrap(), "Clients/Formosa");
    assert_eq!(
        db.all_folder_paths().unwrap(),
        vec![("Clients".to_string(), 0), ("Clients/Formosa".to_string(), 2), ("Servers".to_string(), 1)]
    );

    // Folder names and snippet titles are searchable.
    assert_eq!(search(&db, "formosa"), vec![b, a]);
    assert_eq!(search(&db, "tunnel"), vec![b]);

    let d = db.detail(b).unwrap().unwrap();
    assert_eq!(d.folders, vec!["Clients/Formosa"]);
    assert_eq!(d.title.as_deref(), Some("2. Tunnel"));

    // Moving a clip up into the parent of its only folder.
    let x = insert(&db, 4, "deep one");
    db.set_folders(x, &["X/Y/Z".into()]).unwrap();
    assert_eq!(db.set_folders(x, &["X/Y".into()]).unwrap(), vec!["X/Y"]);
    assert_eq!(db.folder_id("X/Y/Z").unwrap(), None);
    db.delete(x).unwrap();
    assert_eq!(db.folder_id("X").unwrap(), None, "deleting a folder's last clip removes the folder");

    // Taking the last clip out of a folder removes it, and empty parents too.
    db.set_folders(a, &["Clients/Formosa".into()]).unwrap();
    assert_eq!(db.folder_id("Servers").unwrap(), None);
    db.set_folders(a, &[]).unwrap();
    db.set_folders(b, &[]).unwrap();
    assert!(db.folder_entries(0, "").unwrap().is_empty());
    assert!(db.all_folder_paths().unwrap().is_empty());

    // Filed clips survive pruning like starred ones; deleting a clip unfiles it.
    db.set_folders(c, &["Keep".into()]).unwrap();
    assert_eq!(db.clear(Some(i64::MAX), false).unwrap(), 2);
    assert_eq!(db.count().unwrap(), 1);
    db.make_folder("Empty/Tree").unwrap();
    db.clear(None, true).unwrap();
    assert!(db.all_folder_paths().unwrap().is_empty(), "clearing everything leaves no folders");
}

/// A database laid out like Clipy's (its migrations V1, V3 and V4). Clipy
/// registers uuid() itself; the importer opens the file without it.
fn clipy_db(path: &std::path::Path) {
    let c = Connection::open(path).unwrap();
    c.create_scalar_function("uuid", 0, rusqlite::functions::FunctionFlags::SQLITE_UTF8, |_| Ok("x".to_string()))
        .unwrap();
    c.execute_batch(
        r#"
        CREATE TABLE "pasteboardHistories" (
          "id" TEXT PRIMARY KEY NOT NULL ON CONFLICT REPLACE DEFAULT (uuid()),
          "title" TEXT NOT NULL ON CONFLICT REPLACE DEFAULT '',
          "pasteboardTypes" TEXT NOT NULL ON CONFLICT REPLACE DEFAULT '[]',
          "deviceID" TEXT,
          "updateAt" INTEGER NOT NULL ON CONFLICT REPLACE DEFAULT (unixepoch())
        ) STRICT;
        CREATE TABLE "pasteboardHistoryAssets" (
          "id" TEXT PRIMARY KEY NOT NULL ON CONFLICT REPLACE DEFAULT (uuid()),
          "pasteboardHistoryID" TEXT NOT NULL,
          "index" INTEGER NOT NULL ON CONFLICT REPLACE DEFAULT 0,
          "pasteboardType" TEXT NOT NULL ON CONFLICT REPLACE DEFAULT '',
          "data" BLOB NOT NULL,
          FOREIGN KEY ("pasteboardHistoryID") REFERENCES "pasteboardHistories" ("id") ON DELETE CASCADE
        ) STRICT;
        CREATE TABLE "snippetFolders" (
          "id" TEXT PRIMARY KEY NOT NULL ON CONFLICT REPLACE DEFAULT (uuid()),
          "title" TEXT NOT NULL ON CONFLICT REPLACE DEFAULT '',
          "index" INTEGER NOT NULL ON CONFLICT REPLACE DEFAULT 0,
          "isEnabled" INTEGER NOT NULL ON CONFLICT REPLACE DEFAULT 1
        ) STRICT;
        CREATE TABLE "snippets" (
          "id" TEXT PRIMARY KEY NOT NULL ON CONFLICT REPLACE DEFAULT (uuid()),
          "folderID" TEXT NOT NULL,
          "title" TEXT NOT NULL ON CONFLICT REPLACE DEFAULT '',
          "content" TEXT NOT NULL ON CONFLICT REPLACE DEFAULT '',
          "index" INTEGER NOT NULL ON CONFLICT REPLACE DEFAULT 0,
          "isEnabled" INTEGER NOT NULL ON CONFLICT REPLACE DEFAULT 1,
          FOREIGN KEY ("folderID") REFERENCES "snippetFolders" ("id") ON DELETE CASCADE
        ) STRICT;
        ALTER TABLE "pasteboardHistories" ADD COLUMN "createdAt" INTEGER NOT NULL ON CONFLICT REPLACE DEFAULT 0;
        ALTER TABLE "pasteboardHistories" ADD COLUMN "ocrText" TEXT;
        "#,
    )
    .unwrap();
    let hist = |id: &str, t: i64| {
        c.execute(
            r#"INSERT INTO pasteboardHistories(id, title, updateAt, createdAt) VALUES(?, '', ?, ?)"#,
            params![id, t, t - 10],
        )
        .unwrap();
    };
    let asset = |h: &str, i: i64, uti: &str, data: &[u8]| {
        c.execute(
            r#"INSERT INTO pasteboardHistoryAssets(id, pasteboardHistoryID, "index", pasteboardType, data) VALUES(?, ?, ?, ?, ?)"#,
            params![format!("{h}-{i}"), h, i, uti, data],
        )
        .unwrap();
    };
    hist("h1", 1_700_000_000);
    asset("h1", 0, "public.utf8-plain-text", b"hello from clipy");
    asset("h1", 1, "public.rtf", br"{\rtf1 hello from clipy}");
    hist("h2", 1_700_000_100);
    asset("h2", 0, "public.file-url", b"file:///tmp/a.txt");
    asset("h2", 1, "public.file-url", b"file:///tmp/b.txt");
    // Converted from old Clipy: files only as a plist of paths.
    hist("h3", 1_700_000_200);
    let mut names = Vec::new();
    plist::Value::Array(vec!["/tmp/my doc.txt".into()]).to_writer_binary(&mut names).unwrap();
    asset("h3", 0, "NSFilenamesPboardType", &names);
    let folder = |id: &str, title: &str, i: i64| {
        c.execute(r#"INSERT INTO snippetFolders(id, title, "index") VALUES(?, ?, ?)"#, params![id, title, i]).unwrap();
    };
    let snippet = |f: &str, title: &str, content: &str, i: i64| {
        c.execute(
            r#"INSERT INTO snippets(id, folderID, title, content, "index") VALUES(?, ?, ?, ?, ?)"#,
            params![format!("{f}-{i}"), f, title, content, i],
        )
        .unwrap();
    };
    folder("f2", "General", 1);
    folder("f1", "Formosa", 0);
    snippet("f1", "2. VPN Password", "vpn-pass-77", 1);
    snippet("f1", "1. API Key", "api-key-123", 0);
    snippet("f2", "hello", "hello from clipy", 0);
    snippet("f2", "hello again", "hello from clipy", 1);
}

#[test]
fn imports_clipy_database() {
    let dir = std::env::temp_dir().join(format!("crabclip-clipy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sqlite.db");
    let _ = std::fs::remove_file(&path);
    clipy_db(&path);

    let db = Db::open_memory().unwrap();
    let s = Settings::default();
    let st = crabclip::clipy::import(&path, &db, &s, |_| {}).unwrap();
    assert_eq!((st.history, st.added, st.merged, st.skipped), (3, 3, 0, 0));
    assert_eq!((st.folders, st.snippets, st.duplicates), (2, 4, 1));

    // History: the text clip with its RTF, the two files as two items.
    let text = search(&db, "hello from clipy");
    assert_eq!(text.len(), 1, "the snippet with the same text is the same clip");
    let d = db.detail(text[0]).unwrap().unwrap();
    assert_eq!(d.copied_at, 1_700_000_000_000);
    assert!(d.types.contains("public.rtf"));
    assert_eq!(d.folders, vec!["General"]);
    let old = search(&db, "my doc");
    assert_eq!(old.len(), 1, "old Clipy file clips become files");
    assert_eq!(db.detail(old[0]).unwrap().unwrap().kind, Kind::File);
    let files: Vec<i64> = search(&db, "type:file").into_iter().filter(|id| *id != old[0]).collect();
    assert_eq!(files.len(), 1);
    let reps = db.reps(files[0]).unwrap();
    assert_eq!(reps.iter().map(|r| r.item).collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(db.detail(files[0]).unwrap().unwrap().kind, Kind::File);

    // Snippets: Clipy's folders and order, its titles, dated before the history.
    assert_eq!(labels(&db.folder_entries(0, "").unwrap()), vec!["Formosa/ (2)", "General/ (1)"]);
    let general = db.folder_id("General").unwrap().unwrap();
    assert_eq!(labels(&db.folder_entries(general, "").unwrap()), vec!["hello again"], "same text: merged, later title");
    let formosa = db.folder_id("Formosa").unwrap().unwrap();
    assert_eq!(labels(&db.folder_entries(formosa, "").unwrap()), vec!["1. API Key", "2. VPN Password"]);
    let key = search(&db, "api-key-123")[0];
    let kd = db.detail(key).unwrap().unwrap();
    assert!(kd.copied_at < 1_700_000_000_000);
    assert_eq!(kd.app_name.as_deref(), Some("Clipy"));
    assert_eq!(kd.title.as_deref(), Some("1. API Key"));

    // Importing again changes nothing.
    let n = db.count().unwrap();
    let before: Vec<(i64, i64, i64)> = db
        .conn
        .prepare("SELECT created_at, copied_at, copy_count FROM clips ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let st = crabclip::clipy::import(&path, &db, &s, |_| {}).unwrap();
    assert_eq!((st.added, st.merged), (0, 3));
    assert_eq!(db.count().unwrap(), n);
    let after: Vec<(i64, i64, i64)> = db
        .conn
        .prepare("SELECT created_at, copied_at, copy_count FROM clips ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(before, after, "dates and copy counts don't move on a re-import");
    assert_eq!(labels(&db.folder_entries(formosa, "").unwrap()), vec!["1. API Key", "2. VPN Password"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn imports_clipy_snippets_xml() {
    let dir = std::env::temp_dir().join(format!("crabclip-clipy-xml-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("snippets.xml");
    std::fs::write(
        &path,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<folders>
  <folder><title>Formosa</title><snippets>
    <snippet><title>1. API Key</title><content>key &amp; more</content></snippet>
    <snippet><title>ssh test</title><content>ssh me@host</content></snippet>
  </snippets></folder>
  <folder><title>a/b</title><snippets><snippet><title>x</title><content>slash folder</content></snippet></snippets></folder>
</folders>"#,
    )
    .unwrap();
    let db = Db::open_memory().unwrap();
    let st = crabclip::clipy::import(&path, &db, &Settings::default(), |_| {}).unwrap();
    assert_eq!((st.folders, st.snippets), (2, 3));
    let formosa = db.folder_id("Formosa").unwrap().unwrap();
    assert_eq!(labels(&db.folder_entries(formosa, "").unwrap()), vec!["1. API Key", "ssh test"]);
    assert_eq!(search(&db, "key & more").len(), 1);
    assert_eq!(labels(&db.folder_entries(0, "").unwrap()), vec!["Formosa/ (2)", "a∕b/ (1)"], "a / in a name doesn't nest");
    let _ = std::fs::remove_dir_all(&dir);
}
