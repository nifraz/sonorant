//! Access to the Nostalgia+ reference vectors in `tests/reference/`.
//!
//! The vectors were written by Nostalgia+'s test harness (`build\export.cmd <dir>` in
//! that repository). JSON holds the results; `signals/*.f64` hold the analysed audio as
//! interleaved little-endian `f64`, left then right. See `tests/reference/README.md`.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

pub use serde_json::Value;

/// The `tests/reference` directory at the workspace root.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/reference")
}

/// Parses one of the reference JSON files, e.g. `"spectrum.json"`.
pub fn json(name: &str) -> Value {
    let path = root().join(name);
    let text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()))
}

/// A stereo signal as the capture ring held it: `f32` samples.
#[derive(Clone, Debug)]
pub struct Signal {
    pub name: String,
    pub sample_rate: f64,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

/// Loads `signals/<name>.f64`. Every value in those files is exactly an `f32`.
pub fn signal(name: &str, sample_rate: f64) -> Signal {
    let path = root().join("signals").join(format!("{name}.f64"));
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    assert_eq!(
        bytes.len() % 16,
        0,
        "{} is not whole stereo f64 frames",
        path.display()
    );
    let frames = bytes.len() / 16;
    let (mut left, mut right) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
    for frame in bytes.chunks_exact(16) {
        let l = f64::from_le_bytes(frame[..8].try_into().unwrap());
        let r = f64::from_le_bytes(frame[8..].try_into().unwrap());
        assert_eq!(l as f32 as f64, l, "{name}: sample is not an f32");
        assert_eq!(r as f32 as f64, r, "{name}: sample is not an f32");
        left.push(l as f32);
        right.push(r as f32);
    }
    Signal {
        name: name.to_owned(),
        sample_rate,
        left,
        right,
    }
}

/// Convenience accessors for `serde_json::Value`, panicking with the key on a mismatch.
pub trait ValueExt {
    fn f(&self, key: &str) -> f64;
    fn u(&self, key: &str) -> usize;
    fn s(&self, key: &str) -> &str;
    fn b(&self, key: &str) -> bool;
    fn arr(&self, key: &str) -> &Vec<Value>;
    fn floats(&self, key: &str) -> Vec<f64>;
}

impl ValueExt for Value {
    fn f(&self, key: &str) -> f64 {
        self[key]
            .as_f64()
            .unwrap_or_else(|| panic!("{key} is not a number: {}", self[key]))
    }
    fn u(&self, key: &str) -> usize {
        self[key]
            .as_u64()
            .unwrap_or_else(|| panic!("{key} is not an integer: {}", self[key])) as usize
    }
    fn s(&self, key: &str) -> &str {
        self[key]
            .as_str()
            .unwrap_or_else(|| panic!("{key} is not a string: {}", self[key]))
    }
    fn b(&self, key: &str) -> bool {
        self[key]
            .as_bool()
            .unwrap_or_else(|| panic!("{key} is not a bool: {}", self[key]))
    }
    fn arr(&self, key: &str) -> &Vec<Value> {
        self[key]
            .as_array()
            .unwrap_or_else(|| panic!("{key} is not an array"))
    }
    fn floats(&self, key: &str) -> Vec<f64> {
        floats(&self[key])
    }
}

/// A JSON array of numbers as `f64`s.
pub fn floats(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap_or_else(|| panic!("not an array: {v}"))
        .iter()
        .map(|x| x.as_f64().unwrap_or_else(|| panic!("not a number: {x}")))
        .collect()
}

/// Collects mismatches instead of stopping at the first, so a failing test says how
/// bad things are and where.
#[derive(Debug, Default)]
pub struct Mismatches {
    count: usize,
    checked: usize,
    worst: f64,
    report: String,
}

impl Mismatches {
    pub fn new() -> Mismatches {
        Mismatches::default()
    }

    /// Records that `got` differs from `want` by more than `tol` (absolute).
    pub fn close(&mut self, what: impl FnOnce() -> String, got: f64, want: f64, tol: f64) {
        self.checked += 1;
        let d = (got - want).abs();
        if d.is_nan() || d > tol {
            self.count += 1;
            if d > self.worst || d.is_nan() {
                self.worst = d;
            }
            if self.count <= 12 {
                let _ = writeln!(
                    self.report,
                    "  {}: got {got}, want {want} (diff {d:e})",
                    what()
                );
            }
        }
    }

    /// Records an exact mismatch of any comparable value.
    pub fn equal<T: PartialEq + std::fmt::Debug>(
        &mut self,
        what: impl FnOnce() -> String,
        got: T,
        want: T,
    ) {
        self.checked += 1;
        if got != want {
            self.count += 1;
            if self.count <= 12 {
                let _ = writeln!(self.report, "  {}: got {got:?}, want {want:?}", what());
            }
        }
    }

    pub fn checked(&self) -> usize {
        self.checked
    }

    /// Panics with a summary if anything differed.
    pub fn assert_none(&self, label: &str) {
        assert!(
            self.count == 0,
            "{label}: {} of {} values differ (worst {:e}):\n{}",
            self.count,
            self.checked,
            self.worst,
            self.report
        );
    }
}
