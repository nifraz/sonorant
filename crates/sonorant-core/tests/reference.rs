//! Sonorant's model and engine against the values Nostalgia+ produced.

use serde_json::json;
use sonorant_core::dsp::{
    AnalysisQuality, BandAggregate, ChannelPairMode, CurveInterpolation, FilteringAmount,
    FreqScale, WindowType,
};
use sonorant_core::engine::{AnalysisConfig, Engine, Hop, Row, Sink};
use sonorant_core::legacy::{self, LegacySettings};
use sonorant_core::palette::{self, PaletteKind};
use sonorant_core::settings::{Argb, Preset, Settings, ThemeSlot, sanitise_name};
use sonorant_testdata::{Mismatches, Value, ValueExt, floats, json, root, signal};

fn hex(c: palette::Rgb) -> String {
    format!("#{:08X}", c.to_argb())
}

#[test]
fn palettes_match_exactly() {
    let doc = json("palettes.json");
    let mut m = Mismatches::new();
    for p in doc.arr("palettes") {
        let kind = PaletteKind::from_name(p.s("kind")).unwrap();
        assert_eq!(kind.display_name(), p.s("display_name"));
        let lut = palette::build_lut(kind);
        for (i, want) in p.arr("lut").iter().enumerate() {
            m.equal(
                || format!("{kind} [{i}]"),
                hex(lut[i]),
                want.as_str().unwrap().to_owned(),
            );
        }
        for (deg, entries) in p["hue_shifted"].as_object().unwrap() {
            let shifted = palette::build_lut_shifted(kind, deg.parse().unwrap());
            for (i, want) in entries.as_array().unwrap().iter().enumerate() {
                m.equal(
                    || format!("{kind} shifted {deg} [{i}]"),
                    hex(shifted[i]),
                    want.as_str().unwrap().to_owned(),
                );
            }
        }
    }
    m.assert_none("palettes");
}

// ---------------------------------------------------------------- settings files

fn colour(c: Option<Argb>) -> Value {
    json!(c.map_or("auto".to_owned(), |c| c.to_string()))
}

/// Every Nostalgia+ field, by its C# name, as the importer read it.
fn legacy_values(l: &LegacySettings) -> Vec<(&'static str, Value)> {
    let s = &l.settings;
    let t = &s.theme;
    vec![
        ("Preset", json!(s.preset.name())),
        ("Palette", json!(s.palette.name())),
        ("Scale", json!(s.scale.name())),
        ("FMin", json!(s.fmin)),
        ("FMax", json!(s.fmax)),
        ("Quality", json!(s.quality.name())),
        ("Window", json!(s.window.name())),
        ("Aggregate", json!(s.aggregate.name())),
        ("TiltDbPerOctave", json!(s.tilt_db_per_octave)),
        ("AdaptiveRange", json!(s.adaptive_range)),
        ("FloorDb", json!(s.floor_db)),
        ("CeilingDb", json!(s.ceiling_db)),
        ("ShowGrid", json!(s.show_grid)),
        ("ShowLabels", json!(s.show_labels)),
        ("ShowColorBar", json!(s.show_color_bar)),
        ("ShowHud", json!(s.show_hud)),
        ("ShowStatus", json!(s.show_status)),
        ("TargetFps", json!(l.target_fps)),
        ("AttackMs", json!(s.attack_ms)),
        ("ReleaseMs", json!(s.release_ms)),
        ("PeakDecayDbPerSec", json!(s.peak_decay_db_per_sec)),
        ("AverageSeconds", json!(s.average_seconds)),
        ("UseLoopback", json!(l.use_loopback)),
        ("ScrollDivider", json!(l.scroll_divider)),
        ("PairMode", json!(s.pair_mode.name())),
        ("Style", json!(s.style.name())),
        ("Interp", json!(s.interp.name())),
        ("Filter", json!(s.filter.name())),
        ("ShowMax", json!(s.show_max)),
        ("ShowMin", json!(s.show_min)),
        ("ShowAvg", json!(s.show_avg)),
        ("SolidFill", json!(s.solid_fill)),
        ("CurveOnLeft", json!(s.curve_on_left)),
        ("MirrorLeftPane", json!(s.mirror_left_pane)),
        ("CurveWidthPct", json!(s.curve_width_pct)),
        ("WaveHeightPct", json!(s.wave_height_pct)),
        ("Background", json!(s.background.name())),
        ("ShowDbScale", json!(s.show_db_scale)),
        ("ShowTimeMarks", json!(s.show_time_marks)),
        ("ShowSemitones", json!(s.show_semitones)),
        ("ShowOuterLabels", json!(s.show_outer_labels)),
        ("ShowAxisLabels", json!(s.show_axis_labels)),
        ("LabelMode", json!(s.label_mode.name())),
        ("LabelFontSize", json!(s.label_font_size as f64)),
        ("SyncHover", json!(s.sync_hover)),
        ("ShowHoverPin", json!(s.show_hover_pin)),
        ("ShowHarmonics", json!(s.show_harmonics)),
        ("SeekOnImageClick", json!(s.seek_on_image_click)),
        ("ShowOsd", json!(s.show_osd)),
        ("ShowQuickButtons", json!(s.show_quick_buttons)),
        ("QuickBarCompact", json!(s.quick_bar_compact)),
        ("ReserveScaleSpace", json!(s.reserve_scale_space)),
        ("ScaleLanePos", json!(s.scale_lane_pos.name())),
        ("ShowScaleUnits", json!(s.show_scale_units)),
        ("ShowCenterDeck", json!(s.show_center_deck)),
        ("DeckHeightPx", json!(s.deck_height_px)),
        ("DeckShowGoniometer", json!(s.deck_show_goniometer)),
        ("DeckShowTransport", json!(s.deck_show_transport)),
        ("DeckShowArtwork", json!(s.deck_show_artwork)),
        ("DeckShowTrackInfo", json!(s.deck_show_track_info)),
        ("DeckShowCorrelation", json!(s.deck_show_correlation)),
        ("DeckShowBalance", json!(s.deck_show_balance)),
        ("DeckShowLufsM", json!(s.deck_show_lufs_m)),
        ("DeckShowLufsS", json!(s.deck_show_lufs_s)),
        ("DeckShowTruePeak", json!(s.deck_show_true_peak)),
        ("DeckShowCrest", json!(s.deck_show_crest)),
        ("DeckShowLufsI", json!(s.deck_show_lufs_i)),
        ("DeckShowLra", json!(s.deck_show_lra)),
        ("DeckShowOvers", json!(s.deck_show_overs)),
        ("DeckShowBpm", json!(s.deck_show_bpm)),
        ("DeckShowBrightness", json!(s.deck_show_brightness)),
        ("QuickBarSplit", json!(s.quick_bar_split)),
        ("BarSize", json!(s.bar_size)),
        ("LedSegment", json!(s.led_segment)),
        ("DockPanelHeight", json!(l.dock_panel_height)),
        ("Contrast", json!(s.contrast)),
        ("GutterWidth", json!(s.gutter_width)),
        ("ShowWaveform", json!(s.show_waveform)),
        ("Immersive", json!(s.immersive)),
        ("Glow", json!(s.glow)),
        ("AutoHide", json!(s.auto_hide)),
        ("ImmBackdrop", json!(s.imm_backdrop)),
        ("BackdropPct", json!(s.backdrop_pct)),
        ("ImmBeatReactive", json!(s.imm_beat_reactive)),
        ("ImmColourFollows", json!(s.imm_colour_follows)),
        ("ColourFollowDegrees", json!(s.colour_follow_degrees)),
        ("ImmCinematic", json!(s.imm_cinematic)),
        ("ColBackground", colour(t.background)),
        ("ColPanel", colour(t.panel)),
        ("ColGridMajor", colour(t.grid_major)),
        ("ColGridMinor", colour(t.grid_minor)),
        ("ColAxisText", colour(t.axis_text)),
        ("ColUnits", colour(t.units)),
        ("ColCurve", colour(t.curve)),
        ("ColPeakTrace", colour(t.peak_trace)),
        ("ColAverageTrace", colour(t.average_trace)),
        ("ColMinimumTrace", colour(t.minimum_trace)),
        ("ColHover", colour(t.hover)),
        ("ColWaveform", colour(t.waveform)),
        ("ColSnapshot", colour(t.snapshot)),
    ]
}

fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() <= 1e-12 * x.abs().max(1.0),
        _ => a == b,
    }
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap()
}

#[test]
fn settings_files_import_as_nostalgia_plus_read_them() {
    let doc = json("settings.json");
    let mut m = Mismatches::new();
    for file in doc.arr("files") {
        let name = file.s("name");
        let imported = LegacySettings::parse(&read(file.s("file")));
        let ours = legacy_values(&imported);
        let want = file["values"].as_object().unwrap();
        // Every field Nostalgia+ had is accounted for.
        for key in want.keys() {
            assert!(
                ours.iter().any(|(k, _)| k == key),
                "{name}: the importer doesn't cover {key}"
            );
        }
        for (key, got) in &ours {
            let w = &want[*key];
            m.equal(|| format!("{name} {key} = {got}"), same(got, w), true);
        }
    }
    m.assert_none("imported settings");
}

#[test]
fn presets_match_nostalgia_plus() {
    let doc = json("settings.json");
    for file in doc.arr("files") {
        let Some(p) = file.s("name").strip_prefix("preset_") else {
            continue;
        };
        let preset = Preset::from_name(p).unwrap();
        let mut ours = Settings::default();
        ours.apply_preset(preset);
        let theirs = LegacySettings::parse(&read(file.s("file"))).into_settings();
        assert_eq!(ours, theirs, "{p}");
    }
    let default = LegacySettings::parse(&read("settings/default.settings")).into_settings();
    assert_eq!(default, Settings::default());
}

#[test]
fn every_setting_survives_a_save_and_load() {
    // Nostalgia+ checked this by reflection: add a field, forget one of its two lines in
    // Load or Save, and the value silently vanished on restart. Here serde writes every
    // field, and the file with every field changed proves the importer and the TOML
    // round trip both carry all of them.
    let mutated = LegacySettings::parse(&read("settings/mutated.settings")).into_settings();
    let default = Settings::default();
    let a = serde_json::to_value(&mutated).unwrap();
    let d = serde_json::to_value(&default).unwrap();
    // Settings with nothing in a Nostalgia+ file to change them: the frame cap, because
    // TargetFps 67 is still "display", and the ones that are new here. The TOML round
    // trip below still covers all of them.
    let no_counterpart = [
        "frame_cap",
        "px_per_row",
        "smooth_time",
        "deck_phosphor",
        "curve_phosphor",
        "history_minutes",
        "phosphor_ms",
        "phosphor_intensity",
    ];
    for (k, v) in a.as_object().unwrap() {
        if !no_counterpart.contains(&k.as_str()) {
            assert_ne!(v, &d[k], "{k} wasn't changed by the mutated file");
        }
    }
    assert_eq!(Settings::from_toml(&mutated.to_toml()), mutated);
    assert_eq!(Settings::from_toml(&default.to_toml()), default);
}

#[test]
fn themes_and_names_match() {
    let theme = legacy::parse_theme(&read("settings/Themes/Midnight.theme"));
    assert_eq!(theme.get(ThemeSlot::Background), Some(Argb(0xFF0C_1824)));
    assert_eq!(theme.get(ThemeSlot::PeakTrace), Some(Argb(0xFFFA_C828)));
    assert!(
        ThemeSlot::ALL
            .iter()
            .filter(|&&s| theme.get(s).is_some())
            .count()
            == 2
    );

    let doc = json("settings.json");
    for (input, want) in doc["sanitise"].as_object().unwrap() {
        assert_eq!(
            sanitise_name(input),
            want.as_str().unwrap(),
            "sanitising {input:?}"
        );
    }
    for (size, want) in doc["scale_lane_height"].as_object().unwrap() {
        let mut s = Settings::default();
        if size == "reserve_off" {
            s.reserve_scale_space = false;
        } else {
            s.label_font_size = size.parse().unwrap();
        }
        assert_eq!(
            s.scale_lane_height() as i64,
            want.as_i64().unwrap(),
            "font {size}"
        );
    }
}

#[test]
fn scroll_rates_match() {
    let doc = json("timeline.json");
    for row in doc.arr("rows_per_second") {
        let (fps, div, cine) = (
            row.u("target_fps"),
            row.u("scroll_divider"),
            row.b("cinematic"),
        );
        // With a divider of 0 Nostalgia+ skipped the cinematic quartering (0 * 4 is
        // still 0, which it read as 1). The menu never offered 0; Sonorant treats it
        // as 1 throughout.
        if div == 0 && cine {
            continue;
        }
        let text = format!("TargetFps={fps}\nScrollDivider={div}\nImmCinematic={cine}");
        let s = LegacySettings::parse(&text).into_settings();
        let want = row.f("rows_per_second");
        assert!(
            (s.effective_rows_per_second() - want).abs() < 1e-9,
            "{fps}/{div}/{cine}"
        );
    }
}

// ---------------------------------------------------------------- the pipeline

#[derive(Default)]
struct Recorder {
    hops: Vec<[f64; 9]>,
    snapshots: Vec<(u64, Vec<[Vec<f64>; 5]>)>,
    rows: usize,
}

impl Sink for Recorder {
    fn hop(&mut self, h: &Hop<'_>) {
        let f = h.features;
        self.hops.push([
            h.analysed as u8 as f64,
            h.floor_db,
            h.ceiling_db,
            f.flux(),
            f.onset() as u8 as f64,
            f.pulse(),
            f.bpm(),
            f.centroid(),
            h.pushed_row as u8 as f64,
        ]);
        if h.analysed && h.index % 30 == 29 {
            let panes = h
                .panes
                .iter()
                .map(|p| {
                    let e = p.extremes();
                    [
                        p.raw().to_vec(),
                        p.display().to_vec(),
                        e.max().to_vec(),
                        e.min().to_vec(),
                        e.average().to_vec(),
                    ]
                })
                .collect();
            self.snapshots.push((h.index, panes));
        }
    }

    fn row(&mut self, _row: &Row<'_>) {
        self.rows += 1;
    }
}

fn config_for(run: &Value) -> AnalysisConfig {
    let v = &run["settings"];
    let map = &run["map"];
    let cine = v.b("ImmCinematic");
    let divider = (v.f("ScrollDivider").max(1.0)) * if cine { 4.0 } else { 1.0 };
    AnalysisConfig {
        quality: AnalysisQuality::from_name(v.s("Quality")).unwrap(),
        window: WindowType::from_name(v.s("Window")).unwrap(),
        aggregate: BandAggregate::from_name(v.s("Aggregate")).unwrap(),
        tilt_db_per_octave: v.f("TiltDbPerOctave"),
        pair_mode: ChannelPairMode::from_name(v.s("PairMode")).unwrap(),
        interp: CurveInterpolation::from_name(v.s("Interp")).unwrap(),
        filter: FilteringAmount::from_name(v.s("Filter")).unwrap(),
        attack_ms: v.f("AttackMs"),
        release_ms: v.f("ReleaseMs") * if cine { 3.0 } else { 1.0 },
        peak_decay_db_per_sec: v.f("PeakDecayDbPerSec"),
        average_seconds: v.f("AverageSeconds"),
        adaptive_range: v.b("AdaptiveRange"),
        contrast: v.f("Contrast"),
        floor_db: v.f("FloorDb"),
        ceiling_db: v.f("CeilingDb"),
        rows_per_second: v.f("TargetFps") / divider,
        scale: FreqScale::from_name(map.s("scale")).unwrap(),
        columns: map.u("width"),
        fmin: map.f("fmin"),
        fmax: map.f("fmax"),
    }
}

fn compare_db(m: &mut Mismatches, label: &str, got: &[f64], want: &[f64]) {
    assert_eq!(got.len(), want.len(), "{label}: length");
    for (i, (&g, &w)) in got.iter().zip(want).enumerate() {
        if w > -120.0 {
            m.close(|| format!("{label} [{i}]"), g, w, 0.01);
        } else {
            m.close(
                || format!("{label} [{i}] (quiet)"),
                g.max(-120.0),
                -120.0,
                0.02,
            );
        }
    }
}

#[test]
fn the_whole_pipeline_matches() {
    let doc = json("pipeline.json");
    let sig = signal("music48", 48000.0);
    for run in doc.arr("runs") {
        let name = run.s("name");
        let config = config_for(run);
        assert_eq!(config.map(48000.0).width, run["map"].u("width"));
        let mut engine = Engine::new(48000.0, run.f("fps"), config);
        // Nostalgia+'s tempo estimate, octave errors and all.
        engine.features_mut().octave_check = false;
        let mut rec = Recorder::default();
        for (l, r) in sig
            .left
            .chunks(run.u("hop_frames"))
            .zip(sig.right.chunks(run.u("hop_frames")))
        {
            engine.push(l, r, &mut rec);
        }

        let want_hops = run.arr("hops");
        assert_eq!(rec.hops.len(), want_hops.len(), "{name}: hop count");
        let mut m = Mismatches::new();
        let columns = [
            "ok", "floor", "ceiling", "flux", "onset", "pulse", "bpm", "centroid", "pushed",
        ];
        let tolerance = [0.0, 1e-6, 1e-6, 1e-7, 0.0, 1e-7, 0.0, 1e-9, 0.0];
        for (k, (got, want)) in rec.hops.iter().zip(want_hops).enumerate() {
            let want = floats(want);
            for c in 0..9 {
                m.close(
                    || format!("{name} hop {k} {}", columns[c]),
                    got[c],
                    want[c],
                    tolerance[c],
                );
            }
        }
        let want_snaps = run.arr("snapshots");
        assert_eq!(
            rec.snapshots.len(),
            want_snaps.len(),
            "{name}: snapshot count"
        );
        for ((hop, panes), want) in rec.snapshots.iter().zip(want_snaps) {
            assert_eq!(*hop, want.u("hop") as u64);
            for (p, (ours, theirs)) in panes.iter().zip(want.arr("panes")).enumerate() {
                for (i, key) in ["raw", "display", "max", "min", "average"]
                    .iter()
                    .enumerate()
                {
                    compare_db(
                        &mut m,
                        &format!("{name} hop {hop} pane {p} {key}"),
                        &ours[i],
                        &theirs.floats(key),
                    );
                }
            }
        }
        m.assert_none(name);
        assert!(rec.rows > 0, "{name}: no rows");
    }
}
