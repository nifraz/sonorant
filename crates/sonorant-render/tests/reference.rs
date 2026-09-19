//! Pane geometry against Nostalgia+'s, rectangle for rectangle.

use sonorant_core::dsp::ChannelPairMode;
use sonorant_core::settings::Settings;
use sonorant_render::layout::{Rect, ScopeLayout};
use sonorant_testdata::{Value, ValueExt, json};

fn rect(v: &Value) -> Rect {
    let a: Vec<i64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect();
    Rect::new(a[0] as i32, a[1] as i32, a[2] as i32, a[3] as i32)
}

#[test]
fn stereo_view_panes_match() {
    let doc = json("layout.json");
    let cases = doc.arr("stereo_scope_1200x600");
    assert!(!cases.is_empty());
    for case in cases {
        let s = Settings {
            pair_mode: ChannelPairMode::from_name(case.s("pair_mode")).unwrap(),
            mirror_left_pane: case.b("mirror_left_pane"),
            ..Settings::default()
        };
        let l = ScopeLayout::new(Rect::new(0, 0, 1200, 600), &s);
        let id = format!(
            "{} mirror={}",
            case.s("pair_mode"),
            case.b("mirror_left_pane")
        );
        assert_eq!(l.outer_left, rect(&case["outer_left"]), "{id} outer left");
        assert_eq!(
            l.outer_right,
            rect(&case["outer_right"]),
            "{id} outer right"
        );
        assert_eq!(l.gutter, rect(&case["gutter"]), "{id} gutter");
        let panes = case.arr("panes");
        assert_eq!(l.panes.len(), panes.len(), "{id} pane count");
        for (i, (ours, theirs)) in l.panes.iter().zip(panes).enumerate() {
            assert_eq!(ours.label, theirs.s("label"), "{id} pane {i} label");
            assert_eq!(ours.bounds, rect(&theirs["bounds"]), "{id} pane {i} bounds");
            assert_eq!(ours.curve, rect(&theirs["curve"]), "{id} pane {i} curve");
            assert_eq!(
                ours.spectro,
                rect(&theirs["spectro"]),
                "{id} pane {i} spectrogram"
            );
            assert_eq!(ours.lane, rect(&theirs["lane"]), "{id} pane {i} lane");
            assert_eq!(
                ours.curve_on_left,
                theirs.b("curve_on_left"),
                "{id} pane {i} curve side"
            );
        }
        assert_eq!(l.columns(), case["map"].u("width"), "{id} columns");
    }
}
