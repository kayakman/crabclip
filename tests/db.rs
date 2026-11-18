use cliprust::db::{AppInfo, Db, Filter, Inserted, Kind, Rep};
use cliprust::ingest::{Capture, build};
use cliprust::settings::Settings;

fn text_capture(ts: i64, text: &str, app: &str) -> Capture {
    Capture {
        ts,
        reps: vec![
            Rep { item: 0, uti: "public.utf8-plain-text".into(), data: text.as_bytes().to_vec() },
            Rep { item: 0, uti: "NSStringPboardType".into(), data: text.as_bytes().to_vec() },
        ],
        app: AppInfo { bundle: Some(format!("com.test.{app}")), name: Some(app.into()), path: None },
        ..Default::default()
    }
}

fn search(db: &Db, q: &str) -> Vec<i64> {
    db.search(&Filter { query: q, starred: false, app: None }, 1000).unwrap()
}

#[test]
fn insert_dedupe_search_roundtrip() {
    let db = Db::open_memory().unwrap();
    let s = Settings::default();
    let big = "hello world ".repeat(2000);
    let a = db.insert(&build(text_capture(1000, &big, "iTerm2"), &s).unwrap(), 3).unwrap();
    let b = db.insert(&build(text_capture(2000, "Ünïcode snowman ☃ https://x", "Safari"), &s).unwrap(), 3).unwrap();
    let c = db.insert(&build(text_capture(3000, "https://example.com/page", "Chrome"), &s).unwrap(), 3).unwrap();
    assert!(matches!(a, Inserted::New(_)) && matches!(b, Inserted::New(_)));
    assert_eq!(db.detail(c.id()).unwrap().unwrap().kind, Kind::Url);

    // legacy duplicate type dropped, data comes back byte-identical
    let reps = db.reps(a.id()).unwrap();
    assert_eq!(reps.len(), 1);
    assert_eq!(reps[0].data, big.as_bytes());
    let d = db.detail(a.id()).unwrap().unwrap();
    assert!(d.stored_size < d.raw_size / 10, "compressed {} vs {}", d.stored_size, d.raw_size);

    // newest first, substring + unicode + metadata + short terms
    assert_eq!(search(&db, ""), vec![c.id(), b.id(), a.id()]);
    assert_eq!(search(&db, "snowm"), vec![b.id()]);
    assert_eq!(search(&db, "unicode"), vec![b.id()], "trigram is case-insensitive");
    assert_eq!(search(&db, "iterm"), vec![a.id()], "app name is searchable");
    assert_eq!(search(&db, "app:safari"), vec![b.id()]);
    assert_eq!(search(&db, "type:url"), vec![c.id()]);
    assert_eq!(search(&db, "https -example"), vec![b.id()]);
    assert_eq!(search(&db, "☃"), vec![b.id()]);

    // copying the same text again moves it to the top instead of duplicating
    let again = db.insert(&build(text_capture(4000, &big, "Terminal"), &s).unwrap(), 3).unwrap();
    assert_eq!(again, Inserted::Existing(a.id()));
    assert_eq!(search(&db, "")[0], a.id());
    assert_eq!(db.count().unwrap(), 3);

    // stars and filters
    db.set_star(b.id(), true).unwrap();
    assert_eq!(db.search(&Filter { query: "", starred: true, app: None }, 10).unwrap(), vec![b.id()]);
    assert_eq!(db.search(&Filter { query: "", starred: false, app: Some("com.test.Chrome") }, 10).unwrap(), vec![c.id()]);

    // clear keeps starred
    assert_eq!(db.clear(None, false).unwrap(), 2);
    assert_eq!(search(&db, ""), vec![b.id()]);
    assert_eq!(search(&db, "hello"), Vec::<i64>::new(), "fts rows removed with clips");

    // OCR text becomes searchable
    db.set_ocr(b.id(), Some("Save your changes"), 1).unwrap();
    assert_eq!(search(&db, "your chan"), vec![b.id()]);
}

#[test]
fn files_and_skips() {
    let s = Settings::default();
    let cap = Capture {
        ts: 1,
        reps: vec![
            Rep { item: 0, uti: "public.file-url".into(), data: b"file:///Users/rod/My%20Doc.txt".to_vec() },
            Rep { item: 1, uti: "public.file-url".into(), data: b"file:///tmp/b.png".to_vec() },
        ],
        ..Default::default()
    };
    let clip = build(cap, &s).unwrap();
    assert_eq!(clip.kind, Kind::File);
    assert_eq!(clip.text.as_deref(), Some("/Users/rod/My Doc.txt\n/tmp/b.png"));
    assert_eq!(clip.preview, "/Users/rod/My Doc.txt\n/tmp/b.png");

    let secret = Capture {
        ts: 1,
        reps: vec![
            Rep { item: 0, uti: "public.utf8-plain-text".into(), data: b"hunter2".to_vec() },
            Rep { item: 0, uti: "org.nspasteboard.ConcealedType".into(), data: b"".to_vec() },
            Rep { item: 0, uti: "com.agilebits.onepassword".into(), data: b"x".to_vec() },
        ],
        ..Default::default()
    };
    assert!(build(secret, &s).is_none());
}

#[test]
fn image_thumbnail_and_dims() {
    // 4x2 red PNG generated through ImageIO itself from a TIFF-less path:
    // use a tiny known PNG.
    let png: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
        0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
        0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
        0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    let info = cliprust::imageio::info(png).expect("info");
    assert_eq!((info.width, info.height), (1, 1));
    let thumb = cliprust::imageio::thumbnail(png, 760, 300).expect("thumb");
    assert_eq!(&thumb[..2], &[0xFF, 0xD8], "jpeg");
    let cap = Capture {
        ts: 5,
        reps: vec![Rep { item: 0, uti: "public.png".into(), data: png.to_vec() }],
        ..Default::default()
    };
    let clip = build(cap, &Settings::default()).unwrap();
    assert_eq!(clip.kind, Kind::Image);
    assert_eq!(clip.img, Some((1, 1, 1.0)));
    let db = Db::open_memory().unwrap();
    let id = db.insert(&clip, 3).unwrap().id();
    assert_eq!(db.reps(id).unwrap()[0].data, png, "image bytes come back as-is");
    assert_eq!(db.pending_ocr(10).unwrap(), vec![id]);
}
