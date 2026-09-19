//! The egui layer for now: a status line and a provisional context menu.
//!
//! Phase 5 replaces the menu with one built from the menu model; this one exercises the
//! pieces that model will drive: checkboxes, radio groups, submenus and shortcuts.

use sonorant_core::dsp::{ChannelPairMode, FreqScale, LoudnessReadings};
use sonorant_core::palette::PaletteKind;
use sonorant_core::settings::Settings;

use crate::pacing::PacingStats;
use crate::present::PresentCounts;

/// What the menu and keys control.
#[derive(Clone, Debug, PartialEq)]
pub struct UiState {
    pub frozen: bool,
    pub palette: PaletteKind,
    pub pair_mode: ChannelPairMode,
    pub scale: FreqScale,
    pub rows_per_second: u32,
    pub px_per_row: u32,
    pub smooth_time: bool,
    pub show_status: bool,
    pub present_mode: wgpu::PresentMode,
    pub fullscreen: bool,
    pub quit: bool,
}

impl UiState {
    pub fn from_settings(s: &Settings, fullscreen: bool) -> UiState {
        UiState {
            frozen: false,
            palette: s.palette,
            pair_mode: s.pair_mode,
            scale: s.scale,
            rows_per_second: s.rows_per_second.round().max(1.0) as u32,
            px_per_row: 1,
            smooth_time: false,
            show_status: s.show_status,
            present_mode: wgpu::PresentMode::Fifo,
            fullscreen,
            quit: false,
        }
    }
}

/// Everything the status line reports.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub pacing: PacingStats,
    /// What reached the screen, where the platform counts it.
    pub presented: Option<PresentCounts>,
    pub adapter: String,
    pub backend: String,
    pub present_mode: String,
    pub size: [u32; 2],
    pub scale: f64,
    pub rows_written: u64,
    pub capture: String,
    pub loudness: Option<LoudnessReadings>,
    pub bpm: f64,
    /// Audio frames and history rows lost.
    pub dropped: (u64, u64),
}

/// Lays out one frame's UI and returns the area left for the visuals, in points.
pub fn show(
    ui: &mut egui::Ui,
    state: &mut UiState,
    status: &Status,
    present_modes: &[wgpu::PresentMode],
) -> egui::Rect {
    if state.show_status {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(&status.capture);
                if let Some(l) = &status.loudness {
                    ui.separator();
                    ui.monospace(format!(
                        "M {:5.1}  S {:5.1}  I {:5.1} LUFS  TP {:5.1} dBTP",
                        l.momentary, l.short_term, l.integrated, l.true_peak_db
                    ));
                }
                if status.bpm > 0.0 {
                    ui.separator();
                    ui.monospace(format!("{:.0} BPM", status.bpm));
                }
                if status.dropped != (0, 0) {
                    ui.separator();
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!(
                            "dropped {} frames, {} rows",
                            status.dropped.0, status.dropped.1
                        ),
                    );
                }
                ui.separator();
                let p = &status.pacing;
                let refresh = p
                    .refresh_hz
                    .map(|hz| format!(" at {hz:.0} Hz"))
                    .unwrap_or_default();
                // Refreshes that repeated a frame when the swapchain counts them, else
                // an estimate from the frame intervals.
                let missed = status.presented.map_or(p.missed, |c| c.repeated);
                ui.monospace(format!(
                    "{:5.1} fps  p99 {:5.2} ms  missed {missed}{refresh}",
                    p.fps, p.p99_ms
                ));
                ui.separator();
                ui.label(format!(
                    "{} ({}, {})",
                    status.adapter, status.backend, status.present_mode
                ));
                if state.frozen {
                    ui.separator();
                    ui.strong("frozen");
                }
            });
        });
    }

    let mut area = egui::Rect::NOTHING;
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            area = ui.max_rect();
            let response = ui.interact(area, egui::Id::new("visuals"), egui::Sense::click());
            if response.clicked() {
                state.frozen = !state.frozen;
            }
            response.context_menu(|ui| menu(ui, state, present_modes));
        });
    area
}

fn menu(ui: &mut egui::Ui, state: &mut UiState, present_modes: &[wgpu::PresentMode]) {
    ui.menu_button("Channels", |ui| {
        for (mode, label) in [
            (ChannelPairMode::LeftRight, "Left and right"),
            (ChannelPairMode::MidSide, "Mid and side"),
            (ChannelPairMode::LeftOnly, "Left only"),
            (ChannelPairMode::RightOnly, "Right only"),
        ] {
            ui.radio_value(&mut state.pair_mode, mode, label);
        }
    });
    ui.menu_button("Frequency axis", |ui| {
        for (scale, label) in [
            (FreqScale::Note, "Notes"),
            (FreqScale::Log, "Logarithmic"),
            (FreqScale::Linear, "Linear"),
        ] {
            ui.radio_value(&mut state.scale, scale, label);
        }
    });
    ui.menu_button("Palette", |ui| {
        for kind in PaletteKind::ALL {
            ui.radio_value(&mut state.palette, kind, kind.display_name());
        }
    });
    ui.menu_button("Scroll speed", |ui| {
        for rps in [15, 30, 60, 120] {
            ui.radio_value(
                &mut state.rows_per_second,
                rps,
                format!("{rps} rows per second"),
            );
        }
    });
    ui.menu_button("Zoom", |ui| {
        for px in [1, 2, 4, 8] {
            ui.radio_value(&mut state.px_per_row, px, format!("{px} px per row"));
        }
    });
    ui.checkbox(&mut state.smooth_time, "Blend between rows");
    ui.separator();
    ui.checkbox(&mut state.frozen, "Freeze\tSpace or click");
    ui.menu_button("Presentation", |ui| {
        for mode in [
            wgpu::PresentMode::Fifo,
            wgpu::PresentMode::Mailbox,
            wgpu::PresentMode::Immediate,
        ] {
            let available = present_modes.contains(&mode);
            ui.add_enabled_ui(available, |ui| {
                ui.radio_value(&mut state.present_mode, mode, format!("{mode:?}"));
            });
        }
    });
    ui.checkbox(&mut state.show_status, "Status line");
    ui.checkbox(&mut state.fullscreen, "Fullscreen\tF11");
    ui.separator();
    if ui.button("Quit").clicked() {
        state.quit = true;
    }
}
