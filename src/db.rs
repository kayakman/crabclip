//! SQLite storage: one row per clip, every pasteboard representation stored
//! byte-for-byte (zstd-compressed when that helps), and a contentless FTS5
//! trigram index for fast substring search over text, OCR text and metadata.

use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};

pub type Result<T> = rusqlite::Result<T>;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS clips(
  id          INTEGER PRIMARY KEY,
  created_at  INTEGER NOT NULL,          -- unix ms, first time seen
  copied_at   INTEGER NOT NULL,          -- unix ms, last copied/pasted (sort key)
  kind        TEXT NOT NULL,             -- text|url|image|file|other
  hash        BLOB NOT NULL UNIQUE,
  preview     TEXT NOT NULL DEFAULT '',  -- first chars of the text, for the list
  text_z      BLOB,                      -- zstd full text
  text_len    INTEGER NOT NULL DEFAULT 0,
  starred     INTEGER NOT NULL DEFAULT 0,
  copy_count  INTEGER NOT NULL DEFAULT 1,
  paste_count INTEGER NOT NULL DEFAULT 0,
  app_bundle  TEXT,
  app_name    TEXT,
  app_path    TEXT,
  window_title TEXT,
  source_path TEXT,
  source_url  TEXT,
  img_w       INTEGER,
  img_h       INTEGER,
  img_scale   REAL,
  thumb       BLOB,
  ocr_text    TEXT,
  ocr_state   INTEGER NOT NULL DEFAULT 0, -- 0 pending (images) / n.a., 1 done, 2 failed
  types       TEXT NOT NULL DEFAULT '',
  raw_size    INTEGER NOT NULL DEFAULT 0,
  stored_size INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS clips_copied ON clips(copied_at DESC);
CREATE INDEX IF NOT EXISTS clips_star ON clips(starred, copied_at DESC);
CREATE INDEX IF NOT EXISTS clips_app ON clips(app_bundle, copied_at DESC);
CREATE INDEX IF NOT EXISTS clips_ocr ON clips(ocr_state, kind);

CREATE TABLE IF NOT EXISTS reps(
  clip_id  INTEGER NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
  item     INTEGER NOT NULL,             -- pasteboard item index
  uti      TEXT NOT NULL,
  codec    INTEGER NOT NULL,             -- 0 raw, 1 zstd, 2 sealed (vault: inner codec byte + data)
  raw_size INTEGER NOT NULL,
  data     BLOB NOT NULL,
  PRIMARY KEY(clip_id, item, uti)
);

CREATE TABLE IF NOT EXISTS apps(
  key  TEXT PRIMARY KEY,                 -- bundle id, or name when unknown
  name TEXT,
  path TEXT,
  icon BLOB                              -- small PNG, survives app uninstall
);

CREATE TABLE IF NOT EXISTS tags(
  clip_id INTEGER NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
  tag     TEXT NOT NULL,                 -- lowercase; "url" automatic, "password" seals the clip
  PRIMARY KEY(clip_id, tag)
);
CREATE INDEX IF NOT EXISTS tags_tag ON tags(tag, clip_id);

CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(
  body, meta, content='', contentless_delete=1, tokenize='trigram remove_diacritics 1'
);
"#;

/// Max characters of a clip's text that go into the search index.
const INDEX_CHARS: usize = 256 * 1024;
pub const PREVIEW_CHARS: usize = 600;
/// Tag that seals a clip (encrypted, hidden, not indexed).
pub const PASSWORD: &str = "password";
/// Tag every URL clip gets.
pub const URL_TAG: &str = "url";
/// What the list shows for a sealed clip.
pub const SEALED_PREVIEW: &str = "••••••••";

/// "  #My Tag " -> "my-tag"
pub fn normalize_tag(t: &str) -> String {
    t.trim().trim_start_matches('#').split_whitespace().collect::<Vec<_>>().join("-").to_lowercase()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Text,
    Url,
    Image,
    File,
    Other,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Url => "url",
            Kind::Image => "image",
            Kind::File => "file",
            Kind::Other => "other",
        }
    }
    pub fn parse(s: &str) -> Kind {
        match s {
            "text" => Kind::Text,
            "url" => Kind::Url,
            "image" => Kind::Image,
            "file" => Kind::File,
            _ => Kind::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rep {
    pub item: i64,
    pub uti: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct AppInfo {
    pub bundle: Option<String>,
    pub name: Option<String>,
    pub path: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NewClip {
    pub ts: i64,
    pub kind: Kind,
    pub hash: Vec<u8>,
    pub text: Option<String>,
    pub preview: String,
    pub app: AppInfo,
    pub window_title: Option<String>,
    pub source_path: Option<String>,
    pub source_url: Option<String>,
    pub img: Option<(i64, i64, f64)>,
    pub thumb: Option<Vec<u8>>,
    pub types: String,
    pub reps: Vec<Rep>,
    pub starred: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inserted {
    New(i64),
    Existing(i64),
}

impl Inserted {
    pub fn id(self) -> i64 {
        match self {
            Inserted::New(i) | Inserted::Existing(i) => i,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RowInfo {
    pub id: i64,
    pub kind: Kind,
    pub preview: String,
    pub starred: bool,
    pub app_key: String,
    pub app_name: Option<String>,
    pub app_path: Option<String>,
    pub copied_at: i64,
    pub img_w: i64,
    pub img_h: i64,
    pub img_scale: f64,
    pub thumb: Option<Vec<u8>>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Detail {
    pub id: i64,
    pub kind: Kind,
    pub text: Option<String>,
    pub created_at: i64,
    pub copied_at: i64,
    pub app_name: Option<String>,
    pub app_bundle: Option<String>,
    pub window_title: Option<String>,
    pub source_path: Option<String>,
    pub source_url: Option<String>,
    pub ocr_text: Option<String>,
    pub types: String,
    pub raw_size: i64,
    pub stored_size: i64,
    pub copy_count: i64,
    pub paste_count: i64,
    pub img_w: i64,
    pub img_h: i64,
    pub starred: bool,
    pub tags: Vec<String>,
}

impl Detail {
    pub fn sealed(&self) -> bool {
        self.tags.iter().any(|t| t == PASSWORD)
    }
}

#[derive(Clone, Debug)]
pub struct AppRow {
    pub key: String,
    pub name: String,
    pub path: Option<String>,
    pub count: i64,
}

pub struct Filter<'a> {
    pub query: &'a str,
    pub starred: bool,
    pub app: Option<&'a str>,
}

pub fn compress(data: &[u8], level: i32) -> (i64, Vec<u8>) {
    if data.len() >= 64 {
        if let Ok(z) = zstd::bulk::compress(data, level) {
            if z.len() < data.len() {
                return (1, z);
            }
        }
    }
    (0, data.to_vec())
}

pub fn decompress(codec: i64, data: Vec<u8>, raw_size: i64) -> Vec<u8> {
    if codec == 2 {
        let inner = crate::vault::get().and_then(|v| v.open(&data)).unwrap_or_default();
        return match inner.split_first() {
            Some((c, rest)) if *c != 2 => decompress(*c as i64, rest.to_vec(), raw_size),
            _ => Vec::new(),
        };
    }
    if codec == 1 {
        zstd::bulk::decompress(&data, raw_size.max(0) as usize).unwrap_or_default()
    } else {
        data
    }
}

fn unzip_text(z: Option<Vec<u8>>, len: i64) -> Option<String> {
    let mut z = z?;
    if crate::vault::is_sealed(&z) {
        z = crate::vault::get()?.open(&z)?;
    }
    let raw = zstd::bulk::decompress(&z, len.max(0) as usize).ok()?;
    Some(String::from_utf8_lossy(&raw).into_owned())
}

fn split_tags(s: Option<String>) -> Vec<String> {
    let mut v: Vec<String> = s.unwrap_or_default().split(',').filter(|t| !t.is_empty()).map(String::from).collect();
    // The password pill first, then alphabetical.
    v.sort_by_key(|t| (t != PASSWORD, t.clone()));
    v
}

fn truncate_chars(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

pub struct Db {
    pub conn: Connection,
    pub path: Option<PathBuf>,
}

impl Db {
    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var("CLIPRUST_DB") {
            return PathBuf::from(p);
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        Path::new(&home).join("Library/Application Support/cliprust/cliprust.db")
    }

    pub fn open(path: &Path) -> Result<Db> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        Self::init(conn, Some(path.to_path_buf()))
    }

    pub fn open_memory() -> Result<Db> {
        Self::init(Connection::open_in_memory()?, None)
    }

    fn init(conn: Connection, path: Option<PathBuf>) -> Result<Db> {
        conn.busy_timeout(std::time::Duration::from_secs(15))?;
        // auto_vacuum only takes effect before the first table exists.
        conn.execute_batch(
            "PRAGMA auto_vacuum=INCREMENTAL; PRAGMA journal_mode=WAL; \
             PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )?;
        conn.execute_batch(SCHEMA)?;
        let db = Db { conn, path };
        if db.get_setting("tags_v1").is_none() {
            // URL clips from before tags existed get the automatic tag.
            let ids: Vec<i64> = {
                let mut st = db.conn.prepare("SELECT id FROM clips WHERE kind='url'")?;
                st.query_map([], |r| r.get(0))?.collect::<Result<_>>()?
            };
            let tx = db.conn.unchecked_transaction()?;
            for id in ids {
                tx.execute("INSERT OR IGNORE INTO tags(clip_id, tag) VALUES(?, ?)", params![id, URL_TAG])?;
                Self::reindex(&tx, id)?;
            }
            tx.execute("INSERT OR REPLACE INTO settings(key, value) VALUES('tags_v1', '1')", [])?;
            tx.commit()?;
        }
        Ok(db)
    }

    /// Insert a clip, or when the same content already exists bump it to the top.
    /// A repeat copy only replaces the stored representations when it carries at
    /// least as many types (a plain-text re-copy never strips RTF/HTML/app data).
    /// Compression happens before the write lock is taken; the transaction is
    /// IMMEDIATE so a concurrent writer makes us wait (busy_timeout) instead of
    /// failing, and SQLITE_BUSY is retried.
    pub fn insert(&self, c: &NewClip, level: i32) -> Result<Inserted> {
        let reps: Vec<(i64, &str, i64, i64, Vec<u8>)> = c
            .reps
            .iter()
            .map(|r| {
                let (codec, data) = compress(&r.data, level);
                (r.item, r.uti.as_str(), codec, r.data.len() as i64, data)
            })
            .collect();
        let text_z = c.text.as_ref().map(|t| zstd::bulk::compress(t.as_bytes(), level.max(3)).unwrap_or_default());
        // A copy of a sealed secret matches it by its keyed hash.
        let has_secrets: bool = self
            .conn
            .query_row("SELECT EXISTS(SELECT 1 FROM tags WHERE tag=?)", [PASSWORD], |r| r.get(0))?;
        let secret_hash = if has_secrets { crate::vault::get().map(|v| v.secret_hash(&c.hash)) } else { None };
        let mut tries = 0;
        loop {
            match self.insert_tx(c, &reps, text_z.as_deref(), secret_hash.as_deref()) {
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if matches!(e.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
                        && tries < 8 =>
                {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
                r => return r,
            }
        }
    }

    fn insert_tx(
        &self,
        c: &NewClip,
        reps: &[(i64, &str, i64, i64, Vec<u8>)],
        text_z: Option<&[u8]>,
        secret_hash: Option<&[u8]>,
    ) -> Result<Inserted> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, rusqlite::TransactionBehavior::Immediate)?;
        let raw: i64 = reps.iter().map(|r| r.3).sum();
        let stored: i64 = reps.iter().map(|r| r.4.len() as i64).sum();
        let text_len = c.text.as_ref().map(|t| t.len() as i64).unwrap_or(0);
        let existing: Option<(i64, i64, String)> = tx
            .query_row("SELECT id, copied_at, types FROM clips WHERE hash=?", [&c.hash], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        if existing.is_none() {
            if let Some(sh) = secret_hash {
                let sealed: Option<i64> =
                    tx.query_row("SELECT id FROM clips WHERE hash=?", [sh], |r| r.get(0)).optional()?;
                if let Some(id) = sealed {
                    // Keep it sealed: only the copy metadata changes.
                    tx.execute(
                        "UPDATE clips SET copied_at=MAX(copied_at, ?2), copy_count=copy_count+1, \
                         app_bundle=?3, app_name=?4, app_path=?5, window_title=?6 WHERE id=?1",
                        params![id, c.ts, c.app.bundle, c.app.name, c.app.path, c.window_title],
                    )?;
                    Self::reindex(&tx, id)?;
                    tx.commit()?;
                    return Ok(Inserted::Existing(id));
                }
            }
        }

        if let Some((id, copied_at, old_types)) = existing {
            if c.ts >= copied_at {
                tx.execute(
                    "UPDATE clips SET copied_at=?2, copy_count=copy_count+1, \
                     starred=MAX(starred, ?3), app_bundle=?4, app_name=?5, app_path=?6, \
                     window_title=?7, source_path=?8, source_url=COALESCE(?9, source_url) WHERE id=?1",
                    params![
                        id,
                        c.ts,
                        c.starred,
                        c.app.bundle,
                        c.app.name,
                        c.app.path,
                        c.window_title,
                        c.source_path,
                        c.source_url
                    ],
                )?;
                let old_n = old_types.split(',').filter(|t| !t.is_empty()).count();
                let new_n = c.types.split(',').filter(|t| !t.is_empty()).count();
                if new_n >= old_n {
                    Self::write_reps(&tx, id, reps, true)?;
                    tx.execute(
                        "UPDATE clips SET types=?2, raw_size=?3, stored_size=?4, preview=?5, \
                         text_z=?6, text_len=?7 WHERE id=?1",
                        params![id, c.types, raw, stored, c.preview, text_z, text_len],
                    )?;
                }
            } else {
                tx.execute(
                    "UPDATE clips SET copy_count=copy_count+1, starred=MAX(starred, ?2), \
                     created_at=MIN(created_at, ?3) WHERE id=?1",
                    params![id, c.starred, c.ts],
                )?;
            }
            Self::reindex(&tx, id)?;
            tx.commit()?;
            return Ok(Inserted::Existing(id));
        }

        let (w, h, scale) = c.img.unwrap_or((0, 0, 1.0));
        let ocr_state = if c.kind == Kind::Image { 0 } else { 1 };
        tx.execute(
            "INSERT INTO clips(created_at, copied_at, kind, hash, preview, text_z, text_len, \
             starred, app_bundle, app_name, app_path, window_title, source_path, source_url, \
             img_w, img_h, img_scale, thumb, ocr_state, types, raw_size, stored_size) \
             VALUES(?1,?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
            params![
                c.ts,
                c.kind.as_str(),
                c.hash,
                c.preview,
                text_z,
                text_len,
                c.starred,
                c.app.bundle,
                c.app.name,
                c.app.path,
                c.window_title,
                c.source_path,
                c.source_url,
                if c.img.is_some() { Some(w) } else { None },
                if c.img.is_some() { Some(h) } else { None },
                if c.img.is_some() { Some(scale) } else { None },
                c.thumb,
                ocr_state,
                c.types,
                raw,
                stored
            ],
        )?;
        let id = tx.last_insert_rowid();
        Self::write_reps(&tx, id, reps, false)?;
        if c.kind == Kind::Url {
            tx.execute("INSERT OR IGNORE INTO tags(clip_id, tag) VALUES(?, ?)", params![id, URL_TAG])?;
        }
        Self::reindex(&tx, id)?;
        tx.commit()?;
        Ok(Inserted::New(id))
    }

    fn write_reps(tx: &Connection, id: i64, reps: &[(i64, &str, i64, i64, Vec<u8>)], replace: bool) -> Result<()> {
        if replace {
            tx.execute("DELETE FROM reps WHERE clip_id=?", [id])?;
        }
        let mut st = tx.prepare_cached(
            "INSERT OR REPLACE INTO reps(clip_id, item, uti, codec, raw_size, data) VALUES(?,?,?,?,?,?)",
        )?;
        for (item, uti, codec, raw_size, data) in reps {
            st.execute(params![id, item, uti, codec, raw_size, data])?;
        }
        Ok(())
    }

    /// Rebuild the FTS row of a clip from its stored text, OCR text and metadata.
    pub fn reindex(conn: &Connection, id: i64) -> Result<()> {
        let row = conn
            .query_row(
                "SELECT text_z, text_len, ocr_text, app_name, window_title, source_path, \
                 source_url, app_bundle FROM clips WHERE id=?",
                [id],
                |r| {
                    Ok((
                        r.get::<_, Option<Vec<u8>>>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        [
                            r.get::<_, Option<String>>(3)?,
                            r.get::<_, Option<String>>(4)?,
                            r.get::<_, Option<String>>(5)?,
                            r.get::<_, Option<String>>(6)?,
                            r.get::<_, Option<String>>(7)?,
                        ],
                    ))
                },
            )
            .optional()?;
        conn.execute("DELETE FROM fts WHERE rowid=?", [id])?;
        let Some((text_z, len, ocr, meta)) = row else {
            return Ok(());
        };
        let tags = Self::tags_of(conn, id)?;
        // A sealed clip is found by its metadata and tags only.
        let mut body = String::new();
        if !tags.iter().any(|t| t == PASSWORD) {
            let text = unzip_text(text_z, len).unwrap_or_default();
            body = truncate_chars(&text, INDEX_CHARS).to_string();
            if let Some(o) = ocr {
                body.push('\n');
                body.push_str(&o);
            }
        }
        let mut meta = meta.into_iter().flatten().collect::<Vec<_>>();
        meta.extend(tags);
        let meta = meta.join("\n");
        conn.execute("INSERT INTO fts(rowid, body, meta) VALUES(?,?,?)", params![id, body, meta])?;
        Ok(())
    }

    pub fn search(&self, f: &Filter, limit: usize) -> Result<Vec<i64>> {
        let p = crate::query::parse(f.query);
        let mut sql = String::from("SELECT id FROM clips WHERE 1");
        let mut args: Vec<String> = Vec::new();
        let arg = |args: &mut Vec<String>, v: String| {
            args.push(v);
            format!("?{}", args.len())
        };
        if !p.terms.is_empty() {
            let a = arg(&mut args, crate::query::fts_expr(&p.terms));
            sql += &format!(" AND id IN (SELECT rowid FROM fts WHERE fts MATCH {a})");
        }
        for t in &p.not_terms {
            let a = arg(&mut args, crate::query::fts_expr(std::slice::from_ref(t)));
            sql += &format!(" AND id NOT IN (SELECT rowid FROM fts WHERE fts MATCH {a})");
        }
        for s in &p.short {
            let a = arg(&mut args, crate::query::like_pattern(s));
            sql += &format!(" AND (preview LIKE {a} ESCAPE '\\' OR app_name LIKE {a} ESCAPE '\\')");
        }
        if f.starred || p.starred {
            sql += " AND starred=1";
        }
        if let Some(app) = f.app {
            let a = arg(&mut args, app.to_string());
            sql += &format!(" AND COALESCE(app_bundle, app_name, '')={a}");
        }
        if let Some(app) = &p.app {
            let a = arg(&mut args, crate::query::like_pattern(app));
            sql += &format!(" AND (app_name LIKE {a} ESCAPE '\\' OR app_bundle LIKE {a} ESCAPE '\\')");
        }
        for t in &p.tags {
            let a = arg(&mut args, normalize_tag(t));
            sql += &format!(" AND id IN (SELECT clip_id FROM tags WHERE tag={a})");
        }
        if let Some(k) = &p.kind {
            let a = arg(&mut args, k.clone());
            sql += &format!(" AND kind={a}");
        }
        sql += &format!(" ORDER BY copied_at DESC, id DESC LIMIT {limit}");
        let mut st = self.conn.prepare(&sql)?;
        let ids = st
            .query_map(rusqlite::params_from_iter(args.iter()), |r| r.get(0))?
            .collect::<Result<Vec<i64>>>();
        ids
    }

    pub fn row(&self, id: i64) -> Result<Option<RowInfo>> {
        self.conn
            .query_row(
                "SELECT id, kind, preview, starred, COALESCE(app_bundle, app_name, ''), app_name, \
                 app_path, copied_at, COALESCE(img_w,0), COALESCE(img_h,0), COALESCE(img_scale,1), \
                 thumb, (SELECT group_concat(tag, ',') FROM tags WHERE clip_id=clips.id) FROM clips WHERE id=?",
                [id],
                |r| {
                    Ok(RowInfo {
                        id: r.get(0)?,
                        kind: Kind::parse(&r.get::<_, String>(1)?),
                        preview: r.get(2)?,
                        starred: r.get(3)?,
                        app_key: r.get(4)?,
                        app_name: r.get(5)?,
                        app_path: r.get(6)?,
                        copied_at: r.get(7)?,
                        img_w: r.get(8)?,
                        img_h: r.get(9)?,
                        img_scale: r.get(10)?,
                        thumb: r.get(11)?,
                        tags: split_tags(r.get(12)?),
                    })
                },
            )
            .optional()
    }

    pub fn detail(&self, id: i64) -> Result<Option<Detail>> {
        self.conn
            .query_row(
                "SELECT id, kind, text_z, text_len, created_at, copied_at, app_name, app_bundle, \
                 window_title, source_path, source_url, ocr_text, types, raw_size, stored_size, \
                 copy_count, paste_count, COALESCE(img_w,0), COALESCE(img_h,0), starred, \
                 (SELECT group_concat(tag, ',') FROM tags WHERE clip_id=clips.id) FROM clips WHERE id=?",
                [id],
                |r| {
                    Ok(Detail {
                        id: r.get(0)?,
                        kind: Kind::parse(&r.get::<_, String>(1)?),
                        text: unzip_text(r.get(2)?, r.get(3)?),
                        created_at: r.get(4)?,
                        copied_at: r.get(5)?,
                        app_name: r.get(6)?,
                        app_bundle: r.get(7)?,
                        window_title: r.get(8)?,
                        source_path: r.get(9)?,
                        source_url: r.get(10)?,
                        ocr_text: r.get(11)?,
                        types: r.get(12)?,
                        raw_size: r.get(13)?,
                        stored_size: r.get(14)?,
                        copy_count: r.get(15)?,
                        paste_count: r.get(16)?,
                        img_w: r.get(17)?,
                        img_h: r.get(18)?,
                        starred: r.get(19)?,
                        tags: split_tags(r.get(20)?),
                    })
                },
            )
            .optional()
    }

    /// All representations, decompressed, in pasteboard order.
    pub fn reps(&self, id: i64) -> Result<Vec<Rep>> {
        let mut st = self.conn.prepare_cached(
            "SELECT item, uti, codec, raw_size, data FROM reps WHERE clip_id=? ORDER BY item, rowid",
        )?;
        let rows = st.query_map([id], |r| {
            Ok(Rep {
                item: r.get(0)?,
                uti: r.get(1)?,
                data: decompress(r.get(2)?, r.get(4)?, r.get(3)?),
            })
        })?;
        rows.collect()
    }

    pub fn rep(&self, id: i64, uti: &str) -> Result<Option<Vec<u8>>> {
        self.conn
            .query_row(
                "SELECT codec, raw_size, data FROM reps WHERE clip_id=? AND uti=? ORDER BY item LIMIT 1",
                params![id, uti],
                |r| Ok(decompress(r.get(0)?, r.get(2)?, r.get(1)?)),
            )
            .optional()
    }

    pub fn rep_types(&self, id: i64) -> Result<Vec<String>> {
        let mut st = self.conn.prepare_cached("SELECT DISTINCT uti FROM reps WHERE clip_id=?")?;
        let rows = st.query_map([id], |r| r.get(0))?;
        rows.collect()
    }

    pub fn set_star(&self, id: i64, on: bool) -> Result<()> {
        self.conn.execute("UPDATE clips SET starred=? WHERE id=?", params![on, id])?;
        Ok(())
    }

    pub fn touch_pasted(&self, id: i64, ts: i64, move_to_top: bool) -> Result<()> {
        if move_to_top {
            self.conn.execute(
                "UPDATE clips SET paste_count=paste_count+1, copied_at=? WHERE id=?",
                params![ts, id],
            )?;
        } else {
            self.conn
                .execute("UPDATE clips SET paste_count=paste_count+1 WHERE id=?", [id])?;
        }
        Ok(())
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM fts WHERE rowid=?", [id])?;
        tx.execute("DELETE FROM reps WHERE clip_id=?", [id])?;
        tx.execute("DELETE FROM tags WHERE clip_id=?", [id])?;
        tx.execute("DELETE FROM clips WHERE id=?", [id])?;
        tx.commit()
    }

    /// Delete clips last copied before `before_ms` (None: all). Starred clips are
    /// kept unless `include_starred`. Returns the number deleted.
    pub fn clear(&self, before_ms: Option<i64>, include_starred: bool) -> Result<usize> {
        let mut cond = String::from("1");
        if let Some(b) = before_ms {
            cond += &format!(" AND copied_at < {b}");
        }
        if !include_starred {
            cond += " AND starred=0";
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(&format!("DELETE FROM fts WHERE rowid IN (SELECT id FROM clips WHERE {cond})"), [])?;
        tx.execute(
            &format!("DELETE FROM reps WHERE clip_id IN (SELECT id FROM clips WHERE {cond})"),
            [],
        )?;
        tx.execute(
            &format!("DELETE FROM tags WHERE clip_id IN (SELECT id FROM clips WHERE {cond})"),
            [],
        )?;
        let n = tx.execute(&format!("DELETE FROM clips WHERE {cond}"), [])?;
        tx.commit()?;
        Ok(n)
    }

    pub fn vacuum_some(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA incremental_vacuum; PRAGMA wal_checkpoint(TRUNCATE);")
    }

    pub fn apps(&self) -> Result<Vec<AppRow>> {
        let mut st = self.conn.prepare(
            "SELECT COALESCE(app_bundle, app_name, '') k, MAX(app_name), MAX(app_path), COUNT(*), \
             MAX(copied_at) m FROM clips GROUP BY k ORDER BY m DESC",
        )?;
        let rows = st.query_map([], |r| {
            let key: String = r.get(0)?;
            let name: Option<String> = r.get(1)?;
            Ok(AppRow {
                name: name.unwrap_or_else(|| if key.is_empty() { "Unknown".into() } else { key.clone() }),
                key,
                path: r.get(2)?,
                count: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    pub fn has_app(&self, key: &str) -> Result<bool> {
        self.conn
            .query_row("SELECT 1 FROM apps WHERE key=? AND icon IS NOT NULL", [key], |_| Ok(()))
            .optional()
            .map(|o| o.is_some())
    }

    pub fn save_app(&self, key: &str, name: Option<&str>, path: Option<&str>, icon: Option<&[u8]>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO apps(key, name, path, icon) VALUES(?,?,?,?) ON CONFLICT(key) DO UPDATE SET \
             name=COALESCE(excluded.name, name), path=COALESCE(excluded.path, path), \
             icon=COALESCE(excluded.icon, icon)",
            params![key, name, path, icon],
        )?;
        Ok(())
    }

    pub fn app_icon(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.conn
            .query_row("SELECT icon FROM apps WHERE key=?", [key], |r| r.get(0))
            .optional()
            .map(Option::flatten)
    }

    pub fn pending_ocr(&self, limit: usize) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT id FROM clips WHERE ocr_state=0 AND kind='image' ORDER BY copied_at DESC LIMIT {limit}"
        ))?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    pub fn set_ocr(&self, id: i64, text: Option<&str>, state: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE clips SET ocr_text=?, ocr_state=? WHERE id=?",
            params![text.filter(|t| !t.trim().is_empty()), state, id],
        )?;
        Self::reindex(&tx, id)?;
        tx.commit()
    }

    fn tags_of(conn: &Connection, id: i64) -> Result<Vec<String>> {
        let mut st = conn.prepare_cached("SELECT group_concat(tag, ',') FROM tags WHERE clip_id=?")?;
        let s: Option<String> = st.query_row([id], |r| r.get(0))?;
        Ok(split_tags(s))
    }

    pub fn tags(&self, id: i64) -> Result<Vec<String>> {
        Self::tags_of(&self.conn, id)
    }

    /// Every tag in use with its clip count, most used first.
    pub fn all_tags(&self) -> Result<Vec<(String, i64)>> {
        let mut st = self.conn.prepare_cached("SELECT tag, COUNT(*) n FROM tags GROUP BY tag ORDER BY n DESC, tag")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// Replace a clip's tags. Adding "password" seals the clip (contents and text
    /// encrypted, preview/thumbnail/OCR dropped, not indexed); removing it
    /// restores everything. Fails, changing nothing, without the vault key.
    pub fn set_tags(&self, id: i64, tags: &[String]) -> std::result::Result<Vec<String>, String> {
        let mut new: Vec<String> = Vec::new();
        for t in tags.iter().map(|t| normalize_tag(t)).filter(|t| !t.is_empty()) {
            if !new.contains(&t) {
                new.push(t);
            }
        }
        let old = self.tags(id).map_err(|e| e.to_string())?;
        let want = new.iter().any(|t| t == PASSWORD);
        let had = old.iter().any(|t| t == PASSWORD);
        let run = || -> std::result::Result<(), String> {
            let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
            if want != had {
                let v = crate::vault::get().ok_or("no encryption key (Keychain unavailable)")?;
                if want {
                    Self::seal_clip(&tx, id, v).map_err(|e| e.to_string())?;
                } else {
                    Self::unseal_clip(&tx, id, v)?;
                }
            }
            tx.execute("DELETE FROM tags WHERE clip_id=?", [id]).map_err(|e| e.to_string())?;
            for t in &new {
                tx.execute("INSERT INTO tags(clip_id, tag) VALUES(?, ?)", params![id, t]).map_err(|e| e.to_string())?;
            }
            Self::reindex(&tx, id).map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())
        };
        run()?;
        self.tags(id).map_err(|e| e.to_string())
    }

    fn seal_clip(tx: &Connection, id: i64, v: &crate::vault::Vault) -> Result<()> {
        let reps: Vec<(i64, i64, Vec<u8>)> = {
            let mut st = tx.prepare("SELECT rowid, codec, data FROM reps WHERE clip_id=?")?;
            st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_>>()?
        };
        for (rowid, codec, data) in reps {
            if codec == 2 {
                continue;
            }
            let mut inner = Vec::with_capacity(data.len() + 1);
            inner.push(codec as u8);
            inner.extend_from_slice(&data);
            tx.execute("UPDATE reps SET codec=2, data=? WHERE rowid=?", params![v.seal(&inner), rowid])?;
        }
        let (text_z, hash): (Option<Vec<u8>>, Vec<u8>) =
            tx.query_row("SELECT text_z, hash FROM clips WHERE id=?", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let text_z = text_z.map(|z| if crate::vault::is_sealed(&z) { z } else { v.seal(&z) });
        let hash = if hash.starts_with(b"pw:") { hash } else { v.secret_hash(&hash) };
        tx.execute(
            "UPDATE clips SET text_z=?2, hash=?3, preview=?4, thumb=NULL, ocr_text=NULL, ocr_state=1, \
             stored_size=(SELECT COALESCE(SUM(length(data)),0) FROM reps WHERE clip_id=?1) WHERE id=?1",
            params![id, text_z, hash, SEALED_PREVIEW],
        )?;
        Ok(())
    }

    fn unseal_clip(tx: &Connection, id: i64, v: &crate::vault::Vault) -> std::result::Result<(), String> {
        let e = |e: rusqlite::Error| e.to_string();
        let rows: Vec<(i64, i64, String, i64, i64, Vec<u8>)> = {
            let mut st = tx
                .prepare("SELECT rowid, item, uti, codec, raw_size, data FROM reps WHERE clip_id=? ORDER BY item, rowid")
                .map_err(e)?;
            st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
                .map_err(e)?
                .collect::<Result<_>>()
                .map_err(e)?
        };
        let mut reps = Vec::new();
        for (rowid, item, uti, codec, raw_size, data) in rows {
            let (codec, data) = if codec == 2 {
                let inner = v.open(&data).ok_or("cannot decrypt this clip")?;
                let (c, rest) = inner.split_first().ok_or("empty sealed data")?;
                (*c as i64, rest.to_vec())
            } else {
                (codec, data)
            };
            tx.execute("UPDATE reps SET codec=?, data=? WHERE rowid=?", params![codec, data, rowid]).map_err(e)?;
            reps.push(Rep { item, uti, data: decompress(codec, data, raw_size) });
        }
        let (kind, text_z, text_len, types): (String, Option<Vec<u8>>, i64, String) = tx
            .query_row("SELECT kind, text_z, text_len, types FROM clips WHERE id=?", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .map_err(e)?;
        let kind = Kind::parse(&kind);
        let text_z = match text_z {
            Some(z) if crate::vault::is_sealed(&z) => Some(v.open(&z).ok_or("cannot decrypt this clip's text")?),
            z => z,
        };
        let text = unzip_text(text_z.clone(), text_len);
        let preview = match kind {
            Kind::Other => types.replace(',', ", "),
            _ => text.as_deref().map(crate::ingest::preview_of).unwrap_or_default(),
        };
        let mut thumb = None;
        if kind == Kind::Image {
            thumb = crate::ingest::primary_image(&reps).and_then(|p| crate::imageio::thumbnail(&p.data, 760, 300));
        }
        let hash = crate::ingest::content_hash(kind, &reps).ok_or("cannot rebuild the clip's hash")?;
        tx.execute(
            "UPDATE clips SET text_z=?2, hash=?3, preview=?4, thumb=?5, \
             ocr_state=CASE WHEN kind='image' THEN 0 ELSE 1 END, \
             stored_size=(SELECT COALESCE(SUM(length(data)),0) FROM reps WHERE clip_id=?1) WHERE id=?1",
            params![id, text_z, hash, preview, thumb],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
                "the same content is already in the history unsealed".to_string()
            }
            e => e.to_string(),
        })?;
        Ok(())
    }

    pub fn count(&self) -> Result<i64> {
        self.conn.query_row("SELECT COUNT(*) FROM clips", [], |r| r.get(0))
    }

    pub fn file_size(&self) -> i64 {
        let Some(p) = &self.path else { return 0 };
        let mut total = 0;
        for suffix in ["", "-wal"] {
            let f = PathBuf::from(format!("{}{}", p.display(), suffix));
            total += std::fs::metadata(f).map(|m| m.len() as i64).unwrap_or(0);
        }
        total
    }

    pub fn get_setting(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM settings WHERE key=?", [key], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}
