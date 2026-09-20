//! The beat-reactive backdrop: a field of light behind the analysis that moves with the
//! music.
//!
//! Nostalgia+ had nothing here; Phase 4 put the album cover behind the analysis, blurred,
//! and this goes over it. It adds light and never takes any away, so whatever ground is
//! there keeps its shape and the analysis over it reads as it always did.
//!
//! The beat is a phase rather than a flag. `MusicFeatures` gives an onset envelope and a
//! tempo, and an envelope alone makes a backdrop that twitches when the analysis happens
//! to find a hit and sits still the rest of the time. [`BeatPhase`] carries the beat
//! forward at the tempo between onsets and pulls itself back into step at each one, so
//! the rings keep coming in time through a passage the onset detector loses.

use bytemuck::{Pod, Zeroable};

use crate::colour::Rgba;

/// How many ridges the field is made of, at each visual quality.
pub const RIDGES: [u32; 3] = [2, 3, 4];
/// How much of the way an onset pulls the phase back to the downbeat. All the way would
/// jump on every stray hit; a quarter locks on within a few beats and rides through one
/// the detector missed.
const LOCK: f64 = 0.25;
/// Tempos outside this are not a beat to ride: under it a phase is indistinguishable
/// from a drift, and over it the rings arrive faster than they can be seen.
const BPM_RANGE: (f64, f64) = (40.0, 220.0);

/// Where the beat is, carried forward at the tempo and nudged back into step at each
/// onset.
#[derive(Clone, Copy, Debug, Default)]
pub struct BeatPhase {
    phase: f64,
    /// Last frame's onset envelope, so a rise in it can be told from a decay.
    was: f64,
}

impl BeatPhase {
    /// Advances by `dt` seconds at `bpm`, with `pulse` this frame's onset envelope (1 at
    /// a hit, falling away). Returns the phase, 0 at the downbeat and approaching 1
    /// before the next.
    pub fn advance(&mut self, dt: f64, bpm: f64, pulse: f64) -> f64 {
        if dt.is_finite() && dt > 0.0 && (BPM_RANGE.0..=BPM_RANGE.1).contains(&bpm) {
            self.phase = (self.phase + dt * bpm / 60.0).fract();
        }
        // A rise in the envelope is a hit; the decay after it is not, and pulling on
        // every frame of a decay would drag the phase to a standstill.
        if pulse > self.was + 0.1 {
            // Towards whichever side of the downbeat is nearer, so a beat that arrives
            // early is not chased all the way round the clock.
            let off = if self.phase > 0.5 {
                self.phase - 1.0
            } else {
                self.phase
            };
            self.phase = (self.phase - off * LOCK).rem_euclid(1.0);
        }
        self.was = pulse;
        self.phase
    }

    pub fn phase(&self) -> f64 {
        self.phase
    }
}

/// What the backdrop is drawn from this frame.
#[derive(Clone, Copy, Debug)]
pub struct FieldView {
    pub size: (u32, u32),
    /// Seconds since the app started, for the slow drift.
    pub time: f64,
    pub phase: f64,
    /// The onset envelope, 1 at a hit and falling away.
    pub pulse: f64,
    /// Where the music's energy sits, 0 at the bottom of the axis and 1 at the top.
    pub brightness: f64,
    /// How far forward the backdrop comes, 0 to 1.
    pub strength: f64,
    /// Whether the field answers the beat or only drifts.
    pub reactive: bool,
    /// How many ridges the field is made of: two, three or four, by visual quality.
    /// Fewer is a plainer field as well as a cheaper one.
    pub ridges: u32,
    /// The palette's deep and hot colours.
    pub deep: Rgba,
    pub hot: Rgba,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct FieldUniform {
    deep: [f32; 4],
    hot: [f32; 4],
    size: [f32; 2],
    time: f32,
    phase: f32,
    pulse: f32,
    brightness: f32,
    strength: f32,
    reactive: f32,
    ridges: u32,
    /// WGSL rounds a struct up to its own alignment, which the two `vec4`s make 16, so
    /// the shader's idea of this is 80 bytes whatever Rust packs it into. Getting that
    /// wrong is a pipeline that will not create rather than a picture that looks odd,
    /// but only once something is drawn, so the test below says the number out loud.
    _pad: [u32; 3],
}

#[derive(Debug)]
pub struct BackdropPass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl BackdropPass {
    /// A backdrop drawing into `target`, the visuals' floating-point format.
    pub fn new(device: &wgpu::Device, target: wgpu::TextureFormat) -> BackdropPass {
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("backdrop"),
            size: size_of::<FieldUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("backdrop"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<FieldUniform>() as u64),
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("backdrop"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("backdrop"),
            source: wgpu::ShaderSource::Wgsl(include_str!("backdrop.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("backdrop"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        BackdropPass {
            pipeline: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("backdrop"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target,
                        // Light added, never taken away: the cover under it keeps its
                        // shape and the analysis over it reads as it always did.
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent::OVER,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            }),
            uniforms,
            bind_group,
        }
    }

    pub fn draw(&self, queue: &wgpu::Queue, pass: &mut wgpu::RenderPass<'_>, view: &FieldView) {
        if view.strength <= 0.0 {
            return;
        }
        let u = FieldUniform {
            deep: view.deep.to_linear(),
            hot: view.hot.to_linear(),
            size: [view.size.0 as f32, view.size.1 as f32],
            time: view.time as f32,
            phase: view.phase.rem_euclid(1.0) as f32,
            pulse: view.pulse.clamp(0.0, 1.0) as f32,
            brightness: view.brightness.clamp(0.0, 1.0) as f32,
            strength: view.strength.clamp(0.0, 1.0) as f32,
            reactive: f32::from(u8::from(view.reactive)),
            ridges: view.ridges.clamp(2, 4),
            _pad: [0; 3],
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uniform_matches_the_shader_layout() {
        assert_eq!(size_of::<FieldUniform>(), 80);
        assert_eq!(size_of::<FieldUniform>() % 16, 0);
    }

    /// Between onsets the phase is the tempo and nothing else: a beat at 120 takes half
    /// a second to come round.
    #[test]
    fn the_phase_runs_at_the_tempo() {
        let mut beat = BeatPhase::default();
        for _ in 0..30 {
            beat.advance(1.0 / 60.0, 120.0, 0.0);
        }
        assert!(
            beat.phase() > 0.99 || beat.phase() < 0.01,
            "{}",
            beat.phase()
        );
        for _ in 0..15 {
            beat.advance(1.0 / 60.0, 120.0, 0.0);
        }
        assert!((beat.phase() - 0.5).abs() < 0.02, "{}", beat.phase());
    }

    /// A tempo nobody can dance to is no tempo at all, and the field only drifts.
    #[test]
    fn an_unbelievable_tempo_leaves_the_phase_alone() {
        for bpm in [0.0, 12.0, 400.0, f64::NAN] {
            let mut beat = BeatPhase::default();
            for _ in 0..60 {
                beat.advance(1.0 / 60.0, bpm, 0.0);
            }
            assert_eq!(beat.phase(), 0.0, "{bpm} BPM moved the phase");
        }
    }

    /// Hits pull the phase into step, and going on pulling gets it there.
    #[test]
    fn onsets_pull_the_phase_into_step() {
        // A phase a third of a beat late, hit every beat at the right moment.
        let mut beat = BeatPhase {
            phase: 0.33,
            was: 0.0,
        };
        let mut off = Vec::new();
        for beat_number in 0..12 {
            for frame in 0..30 {
                // 120 BPM at 60 frames a second: a hit every thirtieth frame.
                let pulse = if frame == 0 { 1.0 } else { 0.0 };
                let phase = beat.advance(1.0 / 60.0, 120.0, pulse);
                if frame == 0 {
                    off.push((phase.min(1.0 - phase), beat_number));
                }
            }
        }
        let first = off[0].0;
        let last = off.last().unwrap().0;
        assert!(
            last < first / 4.0,
            "{first} at the start against {last} at the end"
        );
        assert!(last < 0.05, "still {last} out of step after twelve beats");
    }

    /// The decay after a hit is not another hit. Pulling on every frame of it would drag
    /// the phase to a standstill, which is the bug this guards.
    #[test]
    fn a_decaying_pulse_is_not_a_new_onset() {
        let mut beat = BeatPhase::default();
        // One hit, then a long decay, over half a beat at 120.
        let mut pulse = 1.0;
        for i in 0..15 {
            beat.advance(1.0 / 60.0, 120.0, if i == 0 { 1.0 } else { pulse });
            pulse *= 0.85;
        }
        // A quarter of a second at 120 BPM is half a beat, and the one hit at the start
        // only pulled a phase that was already at zero.
        assert!((beat.phase() - 0.5).abs() < 0.02, "{}", beat.phase());
    }
}
