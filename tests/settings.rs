use crabclip::db::Db;
use crabclip::settings::Settings;

#[test]
fn view_settings_roundtrip_and_clamp() {
    let db = Db::open_memory().unwrap();
    let d = Settings::load(&db);
    assert_eq!((d.ui_scale, d.font_size, d.preview_open), (100.0, 13.0, true));

    let s = Settings { ui_scale: 150.0, font_size: 16.0, preview_open: false, ..d };
    s.save(&db);
    assert_eq!(Settings::load(&db), s);

    // Hand-edited or corrupt values stay usable.
    db.set_setting("ui_scale", "900").unwrap();
    db.set_setting("font_size", "2").unwrap();
    let c = Settings::load(&db);
    assert_eq!((c.ui_scale, c.font_size), (200.0, 9.0));
    db.set_setting("ui_scale", "NaN").unwrap();
    db.set_setting("font_size", "inf").unwrap();
    let n = Settings::load(&db);
    assert_eq!((n.ui_scale, n.font_size), (100.0, 13.0));
}
