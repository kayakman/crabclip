//! Importer for Collective.app's history archive (an NSKeyedArchiver binary
//! plist: { clippings: [Clipping{data: {uti: bytes}, applicationName,
//! applicationPath, date, favorite, fileURLArray}] }).

use crate::db::{AppInfo, Db, Rep};
use crate::ingest::{self, Capture};
use crate::settings::Settings;
use plist::{Dictionary, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn default_paths() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let dir = Path::new(&home)
        .join("Library/Containers/com.generation-loss.collective/Data/Library/Application Support/Collective");
    ["Collective.generation-loss2", "Collective.generation-loss"]
        .iter()
        .map(|f| dir.join(f))
        .filter(|p| p.exists())
        .collect()
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    pub total: usize,
    pub added: usize,
    pub merged: usize,
    pub skipped: usize,
}

struct Archive<'a> {
    objs: &'a [Value],
}

impl<'a> Archive<'a> {
    fn get(&self, v: &'a Value) -> &'a Value {
        match v {
            Value::Uid(u) => self.objs.get(u.get() as usize).unwrap_or(v),
            _ => v,
        }
    }
    fn dict(&self, v: &'a Value) -> Option<&'a Dictionary> {
        self.get(v).as_dictionary()
    }
    fn string(&self, v: Option<&'a Value>) -> Option<String> {
        match self.get(v?) {
            Value::String(s) if s != "$null" => Some(s.clone()),
            Value::Dictionary(d) => d.get("NS.string").and_then(|s| self.string(Some(s))),
            _ => None,
        }
    }
    fn bytes(&self, v: &'a Value) -> Option<&'a [u8]> {
        match self.get(v) {
            Value::Data(d) => Some(d),
            Value::Dictionary(d) => d.get("NS.data").and_then(|x| self.bytes(x)),
            Value::String(s) => Some(s.as_bytes()),
            _ => None,
        }
    }
    fn array(&self, v: Option<&'a Value>) -> Vec<&'a Value> {
        let Some(v) = v else { return vec![] };
        match self.get(v) {
            Value::Array(a) => a.iter().collect(),
            Value::Dictionary(d) => d.get("NS.objects").and_then(|o| o.as_array()).map(|a| a.iter().collect()).unwrap_or_default(),
            _ => vec![],
        }
    }
}

fn bundle_id(app_path: &str, cache: &mut HashMap<String, Option<String>>) -> Option<String> {
    cache
        .entry(app_path.to_string())
        .or_insert_with(|| {
            let v = Value::from_file(Path::new(app_path).join("Contents/Info.plist")).ok()?;
            v.as_dictionary()?.get("CFBundleIdentifier")?.as_string().map(String::from)
        })
        .clone()
}

pub fn import(path: &Path, db: &Db, settings: &Settings, mut progress: impl FnMut(&Stats)) -> Result<Stats, String> {
    crate::log!("collective: loading {}", path.display());
    let root = Value::from_file(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let top = root.as_dictionary().ok_or("not an archive")?;
    let objs = top.get("$objects").and_then(|o| o.as_array()).ok_or("no $objects")?;
    let ar = Archive { objs };
    let root_obj = top
        .get("$top")
        .and_then(|t| t.as_dictionary())
        .and_then(|t| t.get("root"))
        .ok_or("no root")?;
    let rootd = ar.dict(root_obj).ok_or("bad root")?;
    let keys = ar.array(rootd.get("NS.keys"));
    let vals = ar.array(rootd.get("NS.objects"));
    let idx = keys
        .iter()
        .position(|k| ar.string(Some(k)).as_deref() == Some("clippings"))
        .ok_or("no clippings")?;
    let clips = ar.array(vals.get(idx).copied());
    let mut st = Stats { total: clips.len(), ..Default::default() };
    let mut bundles = HashMap::new();
    let mut seen_apps = std::collections::HashSet::new();

    for cv in clips {
        let Some(c) = ar.dict(cv) else {
            st.skipped += 1;
            continue;
        };
        let mut reps = Vec::new();
        if let Some(d) = c.get("data").and_then(|d| ar.dict(d)) {
            let ks = ar.array(d.get("NS.keys"));
            let vs = ar.array(d.get("NS.objects"));
            for (k, v) in ks.iter().zip(vs.iter()) {
                if let (Some(uti), Some(bytes)) = (ar.string(Some(k)), ar.bytes(v)) {
                    reps.push(Rep { item: 0, uti, data: bytes.to_vec() });
                }
            }
        }
        let urls: Vec<String> = ar
            .array(c.get("fileURLArray"))
            .into_iter()
            .filter_map(|u| {
                let d = ar.dict(u)?;
                ar.string(d.get("NS.relative"))
            })
            .collect();
        if !urls.is_empty() {
            reps.retain(|r| r.uti != "public.file-url");
            for (i, u) in urls.iter().enumerate() {
                reps.push(Rep { item: i as i64, uti: "public.file-url".into(), data: u.as_bytes().to_vec() });
            }
            reps.sort_by_key(|r| r.item);
        }
        let name = ar.string(c.get("applicationName"));
        let app_path = ar.string(c.get("applicationPath"));
        let bundle = app_path.as_deref().and_then(|p| bundle_id(p, &mut bundles));
        let secs = c
            .get("date")
            .and_then(|d| ar.dict(d))
            .and_then(|d| d.get("NS.time"))
            .and_then(|t| t.as_real())
            .unwrap_or(0.0);
        let ts = ((secs + 978_307_200.0) * 1000.0) as i64;
        let starred = c.get("favorite").and_then(|f| ar.get(f).as_boolean()).unwrap_or(false);
        let app = AppInfo { bundle: bundle.clone(), name: name.clone(), path: app_path.clone() };
        let key = bundle.clone().or(name.clone()).unwrap_or_default();
        if !key.is_empty() && seen_apps.insert(key.clone()) && !db.has_app(&key).unwrap_or(false) {
            let icon = app_path.as_deref().and_then(crate::appicon::png_for_path);
            let _ = db.save_app(&key, name.as_deref(), app_path.as_deref(), icon.as_deref());
        }
        let cap = Capture { ts, reps, app, window_title: None, source_path: None, starred };
        match ingest::build(cap, settings) {
            Some(clip) => match db.insert(&clip, settings.zstd_level) {
                Ok(crate::db::Inserted::New(_)) => st.added += 1,
                Ok(crate::db::Inserted::Existing(_)) => st.merged += 1,
                Err(e) => {
                    crate::log!("collective: insert failed: {e}");
                    st.skipped += 1;
                }
            },
            None => st.skipped += 1,
        }
        progress(&st);
    }
    crate::log!("collective: {} -> {:?}", path.display(), st);
    Ok(st)
}
