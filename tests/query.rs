use crabclip::query::{fts_expr, like_pattern, parse};

#[test]
fn parses_operators() {
    let p = parse(r#"foo "bar baz" -qux app:Chrome type:image is:star ab"#);
    assert_eq!(p.terms, vec!["foo", "bar baz"]);
    assert_eq!(p.not_terms, vec!["qux"]);
    assert_eq!(p.short, vec!["ab"]);
    assert_eq!(p.app.as_deref(), Some("chrome"));
    assert_eq!(p.kind.as_deref(), Some("image"));
    assert!(p.starred);
}

#[test]
fn escapes() {
    assert_eq!(fts_expr(&["a\"b c".into(), "xyz".into()]), "\"a\"\"b c\" AND \"xyz\"");
    assert_eq!(like_pattern("5%_\\"), "%5\\%\\_\\\\%");
    assert!(parse("   ").is_empty());
}
