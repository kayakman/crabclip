//! Search query parsing.
//!
//! Free words are AND-ed and matched as substrings anywhere in the clip text, the
//! text recognized in images, the source app, window title, file path and URL.
//! Words of 3+ characters go through the FTS5 trigram index; shorter ones fall back
//! to a LIKE scan of the preview. Supported operators:
//!   "exact phrase"   -word (exclude)   app:chrome   type:image|text|url|file   is:star
//!   tag:name (repeatable: all must match)

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Parsed {
    pub terms: Vec<String>,
    pub not_terms: Vec<String>,
    pub short: Vec<String>,
    pub kind: Option<String>,
    pub app: Option<String>,
    pub starred: bool,
    pub tags: Vec<String>,
}

impl Parsed {
    pub fn is_empty(&self) -> bool {
        *self == Parsed::default()
    }
}

fn tokens(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in q.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn kind_alias(v: &str) -> Option<&'static str> {
    Some(match v {
        "image" | "images" | "img" | "pic" | "picture" | "screenshot" => "image",
        "text" | "txt" => "text",
        "url" | "link" | "links" => "url",
        "file" | "files" => "file",
        "other" => "other",
        _ => return None,
    })
}

pub fn parse(q: &str) -> Parsed {
    let mut p = Parsed::default();
    for tok in tokens(q) {
        let lower = tok.to_lowercase();
        if let Some(v) = lower.strip_prefix("tag:") {
            if !v.is_empty() {
                p.tags.push(v.to_string());
            }
            continue;
        }
        if let Some(v) = lower.strip_prefix("app:") {
            if !v.is_empty() {
                p.app = Some(v.to_string());
            }
            continue;
        }
        if let Some(v) = lower.strip_prefix("type:").or_else(|| lower.strip_prefix("is:")) {
            if matches!(v, "star" | "starred" | "fav" | "favorite") {
                p.starred = true;
                continue;
            }
            if let Some(k) = kind_alias(v) {
                p.kind = Some(k.to_string());
                continue;
            }
        }
        if let Some(neg) = tok.strip_prefix('-') {
            if neg.chars().count() >= 3 {
                p.not_terms.push(neg.to_string());
            }
            continue;
        }
        if tok.chars().count() >= 3 {
            p.terms.push(tok);
        } else {
            p.short.push(tok);
        }
    }
    p
}

/// FTS5 MATCH expression: every term quoted (so punctuation is literal) and AND-ed.
pub fn fts_expr(terms: &[String]) -> String {
    terms
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// LIKE pattern with %, _ and \ escaped (use with ESCAPE '\').
pub fn like_pattern(s: &str) -> String {
    let mut out = String::from("%");
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// A search starting with "/" browses the snippet folders: "/" is the top
/// level, "/Clients/ssh/" a folder, and text after the last "/" filters it.
#[derive(Debug, PartialEq, Eq)]
pub struct FolderQuery {
    pub path: String,
    pub filter: String,
}

pub fn parse_folder_query(q: &str) -> Option<FolderQuery> {
    let rest = q.strip_prefix('/')?;
    let (dir, filter) = match rest.rfind('/') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => ("", rest),
    };
    Some(FolderQuery { path: crate::db::folder_parts(dir).join("/"), filter: filter.trim().to_string() })
}
