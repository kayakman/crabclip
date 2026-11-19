use cliprust::db::{AppInfo, Db, Filter, Inserted, Rep, SEALED_PREVIEW};
use cliprust::ingest::{Capture, build};
use cliprust::settings::Settings;

fn capture(ts: i64, text: &str) -> Capture {
    Capture {
        ts,
        reps: vec![
            Rep { item: 0, uti: "public.utf8-plain-text".into(), data: text.as_bytes().to_vec() },
            Rep { item: 0, uti: "public.rtf".into(), data: format!("{{\\rtf1 {text}}}").into_bytes() },
        ],
        app: AppInfo { bundle: Some("com.test.app".into()), name: Some("App".into()), path: None },
        ..Default::default()
    }
}

fn search(db: &Db, q: &str) -> Vec<i64> {
    db.search(&Filter { query: q, starred: false, app: None }, 100).unwrap()
}

fn insert(db: &Db, ts: i64, text: &str) -> Inserted {
    db.insert(&build(capture(ts, text), &Settings::default()).unwrap(), 3).unwrap()
}

#[test]
fn urls_are_tagged_and_tags_filter() {
    let db = Db::open_memory().unwrap();
    let u = insert(&db, 1, "https://example.com/pricing").id();
    let t = insert(&db, 2, "just some words").id();
    assert_eq!(db.tags(u).unwrap(), vec!["url"]);
    assert!(db.tags(t).unwrap().is_empty());
    assert_eq!(search(&db, "tag:url"), vec![u]);

    let got = db.set_tags(t, &["  #Work Stuff ".into(), "work-stuff".into(), "".into()]).unwrap();
    assert_eq!(got, vec!["work-stuff"]);
    assert_eq!(search(&db, "tag:work-stuff"), vec![t]);
    assert_eq!(search(&db, "work-stuff"), vec![t], "tags are searchable words too");
    assert!(search(&db, "tag:url tag:work-stuff").is_empty());
    assert_eq!(db.all_tags().unwrap(), vec![("url".to_string(), 1), ("work-stuff".to_string(), 1)]);
}

#[test]
fn password_tag_seals_and_unseals() {
    cliprust::vault::use_key([7u8; 32]);
    let db = Db::open_memory().unwrap();
    let secret = "hunter2-correct-horse-battery";
    let id = insert(&db, 1, secret).id();
    let before = db.reps(id).unwrap();

    db.set_tags(id, &["password".into()]).unwrap();
    // Stored encrypted, shown hidden, not searchable by content.
    let (preview, text_z, hash): (String, Vec<u8>, Vec<u8>) = db
        .conn
        .query_row("SELECT preview, text_z, hash FROM clips WHERE id=?", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap();
    assert_eq!(preview, SEALED_PREVIEW);
    assert!(cliprust::vault::is_sealed(&text_z));
    assert!(hash.starts_with(b"pw:"));
    let raw: Vec<Vec<u8>> = {
        let mut st = db.conn.prepare("SELECT data FROM reps WHERE clip_id=?").unwrap();
        st.query_map([id], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
    };
    assert!(raw.iter().all(|d| !d.windows(7).any(|w| w == b"hunter2")), "plain bytes left in reps");
    assert!(search(&db, "hunter2").is_empty());
    assert_eq!(search(&db, "tag:password"), vec![id]);
    // Recall still gives back exactly what was copied.
    assert_eq!(db.reps(id).unwrap(), before);
    assert_eq!(db.detail(id).unwrap().unwrap().text.as_deref(), Some(secret));

    // Copying it again bumps the sealed clip instead of storing it in clear.
    assert_eq!(insert(&db, 5, secret), Inserted::Existing(id));
    assert_eq!(db.count().unwrap(), 1);
    assert!(search(&db, "hunter2").is_empty());

    // Removing the tag restores it.
    db.set_tags(id, &[]).unwrap();
    assert_eq!(db.row(id).unwrap().unwrap().preview, secret);
    assert_eq!(search(&db, "hunter2"), vec![id]);
    assert_eq!(db.reps(id).unwrap(), before);
    assert_eq!(insert(&db, 9, secret), Inserted::Existing(id));
}
