//! How long the sound still has to travel after capture has tapped it.
//!
//! Capture reads the mix before the hardware plays it, so the picture runs early by
//! whatever the output path costs. PipeWire knows that figure and publishes it: every
//! sink carries a `Latency` parameter, and the entry for its input side says how long
//! after a player hands over a buffer the sound is heard.
//!
//! Finding which sink is a matter of walking the graph. A monitor capture is linked
//! straight to the sink, so the node feeding us is the sink itself. Capturing one app
//! taps that app's own output stream instead, and the sink is one hop further on, where
//! that stream plays. Both are the same walk, just of different lengths, and both are
//! kept up to date as links come and go, because the sink changes when headphones are
//! plugged in and the delay changes with it.
//!
//! What the sink reports is the graph's own cost: a quantum of it, some samples of
//! headroom, and any fixed delay a filter declares. It is not the whole journey to the
//! ear, which no software can know: an HDMI display or a Bluetooth receiver adds its
//! own, and nothing on the wire says how much. It is the part that moves when the
//! output changes, which is the part worth following.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pipewire as pw;
use pw::spa;
use pw::types::ObjectType;
use spa::param::ParamType;
use spa::pod::Value;

/// A latency the way SPA states one: so many quanta, so many samples, and so many
/// nanoseconds, added together.
///
/// The three are kept apart rather than added on arrival because only the last is an
/// absolute time. A quantum is however many frames the graph is running in now, which
/// changes under the sink's feet as clients come and go, so the figure has to be
/// resolved against the graph as it is when it is asked for, not as it was when the
/// parameter arrived.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Latency {
    pub quanta: f64,
    pub samples: u32,
    pub nanos: u64,
}

impl Latency {
    /// The whole of it in seconds, for a graph running `quantum` frames at `rate`.
    pub fn seconds(self, quantum: u32, rate: u32) -> f64 {
        let per_frame = if rate > 0 { 1.0 / f64::from(rate) } else { 0.0 };
        self.quanta * f64::from(quantum) * per_frame
            + f64::from(self.samples) * per_frame
            + self.nanos as f64 / 1e9
    }

    /// Reads the input side of a sink's `Latency` parameter.
    ///
    /// Each direction arrives as a parameter of its own, so a pod for the output side
    /// (the monitor ports, which is where we are reading from, and which therefore
    /// costs nothing) is not an error, just not the one wanted.
    fn parse(pod: &spa::pod::Pod) -> Option<Latency> {
        let (_, value) =
            spa::pod::deserialize::PodDeserializer::deserialize_any_from(pod.as_bytes()).ok()?;
        let Value::Object(object) = value else {
            return None;
        };
        let mut wanted = false;
        let mut out = Latency::default();
        for p in &object.properties {
            match (p.key, &p.value) {
                (spa::sys::SPA_PARAM_LATENCY_direction, Value::Id(id)) => {
                    wanted = id.0 == spa::sys::SPA_DIRECTION_INPUT;
                }
                // The upper bound of each, which is what anything lining a picture up
                // with a sound wants: the longest the path can take, not the shortest.
                (spa::sys::SPA_PARAM_LATENCY_maxQuantum, Value::Float(v)) => {
                    out.quanta = f64::from(*v);
                }
                (spa::sys::SPA_PARAM_LATENCY_maxRate, Value::Int(v)) => {
                    out.samples = (*v).max(0) as u32;
                }
                (spa::sys::SPA_PARAM_LATENCY_maxNs, Value::Long(v)) => {
                    out.nanos = (*v).max(0) as u64;
                }
                _ => {}
            }
        }
        wanted.then_some(out)
    }
}

/// What the graph looks like, as far as finding the sink needs.
#[derive(Debug, Default)]
struct Graph {
    /// Our own capture stream's node, once it has one.
    ours: Option<u32>,
    /// Every sink, and the last latency it reported.
    sinks: HashMap<u32, Option<Latency>>,
    /// Links by the link's own id, as (the node feeding, the node fed).
    links: HashMap<u32, (u32, u32)>,
}

impl Graph {
    /// The sink the sound we are capturing goes out through.
    fn sink(&self) -> Option<u32> {
        let ours = self.ours?;
        let feeding_us = || self.links.values().filter(move |&&(_, to)| to == ours);
        // On a monitor capture the node feeding us is the sink.
        if let Some(&(sink, _)) = feeding_us().find(|&&(from, _)| self.sinks.contains_key(&from)) {
            return Some(sink);
        }
        // Capturing one app: the sink is where that app's stream plays.
        feeding_us().find_map(|&(app, _)| {
            self.links
                .values()
                .find_map(|&(from, to)| (from == app && self.sinks.contains_key(&to)).then_some(to))
        })
    }

    fn latency(&self) -> Option<Latency> {
        self.sinks.get(&self.sink()?).copied().flatten()
    }
}

/// Watches the graph for the sink the captured sound plays through.
///
/// The proxies and listeners are held here and nowhere else: a bound node stops sending
/// its parameters the moment its proxy is dropped, so the watch has to outlive the
/// callback that set it up.
pub struct SinkDelay {
    graph: Rc<RefCell<Graph>>,
    watched: Rc<RefCell<HashMap<u32, (pw::node::Node, pw::node::NodeListener)>>>,
    _listener: pw::registry::Listener,
    _registry: pw::registry::RegistryRc,
}

impl std::fmt::Debug for SinkDelay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SinkDelay")
            .field("graph", &self.graph.borrow())
            .finish()
    }
}

impl SinkDelay {
    /// Starts watching. Nothing is known until the graph has answered, which is a
    /// round trip away, so the first frames are captured with no figure at all.
    pub fn watch(registry: pw::registry::RegistryRc) -> SinkDelay {
        let graph = Rc::new(RefCell::new(Graph::default()));
        let watched = Rc::new(RefCell::new(HashMap::new()));
        let listener = registry
            .add_listener_local()
            .global({
                let (graph, watched, registry) = (graph.clone(), watched.clone(), registry.clone());
                move |global| match global.type_ {
                    ObjectType::Node => {
                        let Some(props) = global.props else { return };
                        if props.get("media.class") != Some("Audio/Sink") {
                            return;
                        }
                        graph.borrow_mut().sinks.entry(global.id).or_insert(None);
                        let Ok(node) = registry.bind::<pw::node::Node, _>(global) else {
                            return;
                        };
                        let id = global.id;
                        let node_listener = node
                            .add_listener_local()
                            .param({
                                let graph = graph.clone();
                                move |_seq, kind, _index, _next, pod| {
                                    if kind != ParamType::Latency {
                                        return;
                                    }
                                    if let Some(l) = pod.and_then(Latency::parse) {
                                        graph.borrow_mut().sinks.insert(id, Some(l));
                                    }
                                }
                            })
                            .register();
                        node.subscribe_params(&[ParamType::Latency]);
                        watched.borrow_mut().insert(id, (node, node_listener));
                    }
                    ObjectType::Link => {
                        let Some(props) = global.props else { return };
                        let node = |key| props.get(key).and_then(|v| v.parse::<u32>().ok());
                        if let (Some(from), Some(to)) =
                            (node("link.output.node"), node("link.input.node"))
                        {
                            graph.borrow_mut().links.insert(global.id, (from, to));
                        }
                    }
                    _ => {}
                }
            })
            .global_remove({
                let (graph, watched) = (graph.clone(), watched.clone());
                move |id| {
                    let mut g = graph.borrow_mut();
                    g.sinks.remove(&id);
                    g.links.remove(&id);
                    drop(g);
                    watched.borrow_mut().remove(&id);
                }
            })
            .register();
        SinkDelay {
            graph,
            watched,
            _listener: listener,
            _registry: registry,
        }
    }

    /// Says which node the capture stream is, once it has one. Nothing can be found
    /// before this: the walk starts here.
    pub fn ours_is(&self, node: u32) {
        self.graph.borrow_mut().ours = Some(node);
    }

    /// What the sink says its latency is, in seconds, for a graph running `quantum`
    /// frames at `rate`. `None` until a sink has been found and has answered.
    pub fn seconds(&self, quantum: u32, rate: u32) -> Option<f64> {
        Some(self.graph.borrow().latency()?.seconds(quantum, rate))
    }

    /// How many sinks are being watched, for the log line that says what was found.
    pub fn sinks(&self) -> usize {
        self.watched.borrow().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(ours: u32, sinks: &[u32], links: &[(u32, u32)]) -> Graph {
        Graph {
            ours: Some(ours),
            sinks: sinks
                .iter()
                .map(|&s| (s, Some(Latency::default())))
                .collect(),
            links: links
                .iter()
                .copied()
                .enumerate()
                .map(|(i, l)| (i as u32 + 100, l))
                .collect(),
        }
    }

    #[test]
    fn a_monitor_capture_finds_the_sink_it_is_on() {
        // The sink feeds us directly: one hop.
        let g = graph(7, &[3], &[(3, 7)]);
        assert_eq!(g.sink(), Some(3));
    }

    #[test]
    fn capturing_one_app_finds_the_sink_that_app_plays_through() {
        // The app's stream feeds us and also feeds the sink: two hops, and the hop
        // that is not a sink must not be mistaken for one.
        let g = graph(7, &[3], &[(5, 7), (5, 3)]);
        assert_eq!(g.sink(), Some(3));
    }

    #[test]
    fn nothing_is_found_before_the_graph_has_answered() {
        let mut g = graph(7, &[3], &[(3, 7)]);
        g.ours = None;
        assert_eq!(g.sink(), None);
        // A sink that has not reported yet is found but has no figure.
        let mut g = graph(7, &[3], &[(3, 7)]);
        g.sinks.insert(3, None);
        assert_eq!(g.sink(), Some(3));
        assert_eq!(g.latency(), None);
        // Nothing feeding us at all: a stream that is connected to nowhere.
        assert_eq!(graph(7, &[3], &[(3, 9)]).sink(), None);
    }

    #[test]
    fn the_three_parts_of_a_latency_add_up() {
        // A quantum of 1024 at 48 kHz is 21.33 ms, plus 24 samples of headroom.
        let l = Latency {
            quanta: 1.0,
            samples: 24,
            nanos: 0,
        };
        assert!((l.seconds(1024, 48000) - 1048.0 / 48000.0).abs() < 1e-12);
        // The quantum term follows the graph, the rest does not.
        assert!((l.seconds(256, 48000) - 280.0 / 48000.0).abs() < 1e-12);
        // Nanoseconds are an absolute delay a filter declares, and follow nothing.
        let l = Latency {
            quanta: 0.0,
            samples: 0,
            nanos: 5_000_000,
        };
        assert!((l.seconds(1024, 48000) - 0.005).abs() < 1e-12);
        // No rate yet: only the part that is already a time can be believed.
        assert!((l.seconds(1024, 0) - 0.005).abs() < 1e-12);
    }
}
