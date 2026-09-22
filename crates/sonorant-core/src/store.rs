//! Settings, user presets and themes on disk.
//!
//! One folder holds everything: `settings.toml`, a `presets/` folder of whole settings,
//! and a `themes/` folder of colour slots alone, so a theme can go on top of any
//! preset without dragging its analysis settings along.
//!
//! Loading never fails the app: a missing or unreadable file gives the defaults, and a
//! file with some bad values keeps the good ones.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::legacy::{self, LegacySettings};
use crate::settings::{Settings, Theme, sanitise_name};

const SETTINGS_FILE: &str = "settings.toml";
const PRESETS_DIR: &str = "presets";
const THEMES_DIR: &str = "themes";
const EXT: &str = "toml";

/// The folder Sonorant keeps its settings in: `$XDG_CONFIG_HOME/sonorant` (or
/// `~/.config/sonorant`) on Linux, `%APPDATA%\Sonorant` on Windows.
pub fn default_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| Path::new(&a).join("Sonorant"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))
            .map(|c| c.join("sonorant"))
    }
}

impl Settings {
    /// The settings as a TOML document.
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("settings always serialise")
    }

    /// Reads a TOML document. Fields that are missing take their defaults; a field with
    /// a bad value keeps its default and the rest still load.
    pub fn from_toml(text: &str) -> Settings {
        if let Ok(s) = toml::from_str::<Settings>(text) {
            return s;
        }
        // Something didn't read. Keep every key that reads on its own and drop the rest.
        let Ok(table) = text.parse::<toml::Table>() else {
            return Settings::default();
        };
        let reads = |k: &String, v: &toml::Value| {
            let mut probe = toml::Table::new();
            probe.insert(k.clone(), v.clone());
            toml::Value::Table(probe).try_into::<Settings>().is_ok()
        };
        let good: toml::Table = table.into_iter().filter(|(k, v)| reads(k, v)).collect();
        toml::Value::Table(good).try_into().unwrap_or_default()
    }
}

/// Where settings, presets and themes live.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Store {
        Store { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn settings_path(&self) -> PathBuf {
        self.dir.join(SETTINGS_FILE)
    }

    /// The saved settings, or the defaults.
    pub fn load(&self) -> Settings {
        fs::read_to_string(self.settings_path())
            .map(|t| Settings::from_toml(&t))
            .unwrap_or_default()
    }

    /// Whether settings have been saved here before.
    pub fn exists(&self) -> bool {
        self.settings_path().is_file()
    }

    pub fn save(&self, s: &Settings) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        write_atomically(&self.settings_path(), &s.to_toml())
    }

    // ---- user presets ----

    fn preset_path(&self, name: &str) -> Option<PathBuf> {
        let name = sanitise_name(name);
        (!name.is_empty()).then(|| self.dir.join(PRESETS_DIR).join(format!("{name}.{EXT}")))
    }

    /// Saved preset names, sorted without regard to case.
    pub fn list_presets(&self) -> Vec<String> {
        list(&self.dir.join(PRESETS_DIR))
    }

    /// Saves the whole of `s` under `name`. False if the name has nothing usable in it.
    pub fn save_preset(&self, name: &str, s: &Settings) -> io::Result<bool> {
        let Some(path) = self.preset_path(name) else {
            return Ok(false);
        };
        fs::create_dir_all(path.parent().expect("preset paths have a folder"))?;
        write_atomically(&path, &s.to_toml())?;
        Ok(true)
    }

    /// The preset saved under `name`, if there is one.
    pub fn load_preset(&self, name: &str) -> Option<Settings> {
        let path = self.preset_path(name)?;
        fs::read_to_string(path)
            .ok()
            .map(|t| Settings::from_toml(&t))
    }

    pub fn delete_preset(&self, name: &str) -> bool {
        self.preset_path(name)
            .is_some_and(|p| fs::remove_file(p).is_ok())
    }

    // ---- themes ----

    fn theme_path(&self, name: &str) -> Option<PathBuf> {
        let name = sanitise_name(name);
        (!name.is_empty()).then(|| self.dir.join(THEMES_DIR).join(format!("{name}.{EXT}")))
    }

    pub fn list_themes(&self) -> Vec<String> {
        list(&self.dir.join(THEMES_DIR))
    }

    pub fn save_theme(&self, name: &str, theme: &Theme) -> io::Result<bool> {
        let Some(path) = self.theme_path(name) else {
            return Ok(false);
        };
        fs::create_dir_all(path.parent().expect("theme paths have a folder"))?;
        let text = toml::to_string_pretty(theme).expect("themes always serialise");
        write_atomically(&path, &text)?;
        Ok(true)
    }

    pub fn load_theme(&self, name: &str) -> Option<Theme> {
        let text = fs::read_to_string(self.theme_path(name)?).ok()?;
        toml::from_str(&text).ok()
    }

    pub fn delete_theme(&self, name: &str) -> bool {
        self.theme_path(name)
            .is_some_and(|p| fs::remove_file(p).is_ok())
    }

    // ---- Nostalgia+ ----

    /// Brings over Nostalgia+'s settings, user presets and themes from `from`, a
    /// `NostalgiaPlus` folder. Returns the imported settings, or `None` if there were
    /// none there. Presets and themes that already exist here are left alone.
    pub fn import_nostalgia_plus(&self, from: &Path) -> io::Result<Option<Settings>> {
        let Ok(text) = fs::read_to_string(from.join("NostalgiaPlus.settings")) else {
            return Ok(None);
        };
        let settings = LegacySettings::parse(&text).into_settings();
        self.save(&settings)?;
        for (sub, ext) in [("Presets", "settings"), ("Themes", "theme")] {
            let Ok(entries) = fs::read_dir(from.join(sub)) else {
                continue;
            };
            for e in entries.flatten() {
                let path = e.path();
                if path.extension().and_then(|x| x.to_str()) != Some(ext) {
                    continue;
                }
                let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                if ext == "settings" {
                    if self.load_preset(name).is_none() {
                        self.save_preset(name, &LegacySettings::parse(&text).into_settings())?;
                    }
                } else if self.load_theme(name).is_none() {
                    self.save_theme(name, &legacy::parse_theme(&text))?;
                }
            }
        }
        Ok(Some(settings))
    }
}

fn list(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some(EXT))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_owned))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Writes through a temporary file and a rename, so a crash mid-write can't leave a
/// half-written settings file behind.
fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Argb, Preset, ThemeSlot};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let n = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir()
                .join(format!("sonorant-test-{tag}-{}-{n}", std::process::id()));
            fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn settings_round_trip() {
        let dir = TempDir::new("settings");
        let store = Store::new(&dir.0);
        assert!(!store.exists());
        assert_eq!(store.load(), Settings::default());
        let mut s = Settings::default();
        s.apply_preset(Preset::Bass);
        s.contrast = 0.71;
        s.label_font_size = 11.0;
        s.theme.set(ThemeSlot::Hover, Some(Argb(0x80FF_0000)));
        store.save(&s).unwrap();
        assert!(store.exists());
        assert_eq!(store.load(), s);
    }

    #[test]
    fn user_presets_round_trip() {
        // Nostalgia+'s "user preset round trip" section.
        let dir = TempDir::new("presets");
        let store = Store::new(&dir.0);
        let mut a = Settings::default();
        a.apply_preset(Preset::Bass);
        a.contrast = 0.71;
        a.label_font_size = 11.0;
        a.mirror_left_pane = true;
        a.curve_width_pct = 33;
        assert!(store.save_preset("Round Trip", &a).unwrap());
        assert_eq!(store.list_presets(), ["Round Trip"]);

        let b = store.load_preset("Round Trip").unwrap();
        assert_eq!(b, a);
        assert_eq!(b.fmax, 800.0);
        assert!(!store.save_preset("*/?", &a).unwrap());
        assert!(store.delete_preset("Round Trip"));
        assert!(store.list_presets().is_empty());
    }

    #[test]
    fn themes_carry_colours_only() {
        // Nostalgia+'s "themes" section: a theme goes over any preset untouched.
        let dir = TempDir::new("themes");
        let store = Store::new(&dir.0);
        let mut a = Settings::default();
        a.theme.set(ThemeSlot::Background, Some(Argb(0xFF0C_1824)));
        a.theme.set(ThemeSlot::PeakTrace, Some(Argb(0xFFFA_C828)));
        assert!(store.save_theme("Midnight", &a.theme).unwrap());
        assert_eq!(store.list_themes().len(), 1);

        let mut b = Settings::default();
        b.apply_preset(Preset::Bass);
        let (fmax, quality) = (b.fmax, b.quality);
        b.theme.set(ThemeSlot::Background, Some(Argb(0xFF5A_0000)));
        b.theme = store.load_theme("Midnight").unwrap();
        assert_eq!(b.theme.get(ThemeSlot::Background), Some(Argb(0xFF0C_1824)));
        assert_eq!(b.theme.get(ThemeSlot::PeakTrace), Some(Argb(0xFFFA_C828)));
        assert_eq!(b.theme.get(ThemeSlot::Curve), None);
        assert_eq!((b.fmax, b.quality), (fmax, quality));
        b.theme.clear();
        assert!(b.theme.get(ThemeSlot::Background).is_none());
        assert!(store.delete_theme("Midnight") && store.list_themes().is_empty());
    }

    #[test]
    fn a_bad_value_loses_only_itself() {
        let d = Settings::default();
        let mut text = d.to_toml();
        text = text.replace(
            &format!("contrast = {:?}", d.contrast),
            "contrast = \"lots\"",
        );
        text = text.replace(
            &format!("palette = {:?}", d.palette.name()),
            "palette = \"Viridis\"",
        );
        let s = Settings::from_toml(&text);
        assert_eq!(s.contrast, d.contrast, "a bad number falls back");
        assert_eq!(s.palette, crate::palette::PaletteKind::Viridis);
        assert_eq!(
            Settings::from_toml("not toml at all ["),
            Settings::default()
        );
        assert_eq!(Settings::from_toml(""), Settings::default());
    }
}
