//! A file's waveform, with what the analysis found drawn over it: every bar,
//! every eight-bar phrase, the sections, the drops, and whatever spans the view wants
//! shown. A click plays the file from there; player.js does the playing.

use super::{player, State};
use crate::work::{self, PEAKS};
use dioxus::prelude::*;
use std::path::PathBuf;

/// A stretch to shade, seconds, and how: `out` or `into`.
#[derive(Clone, PartialEq)]
pub struct Span {
    pub from: f64,
    pub to: f64,
    pub kind: &'static str,
}

#[derive(Props, Clone, PartialEq)]
pub struct WaveProps {
    pub path: PathBuf,
    pub seconds: f64,
    pub title: String,
    #[props(default)]
    pub downbeats: Vec<f64>,
    #[props(default)]
    pub phrases: Vec<f64>,
    #[props(default)]
    pub sections: Vec<f64>,
    #[props(default)]
    pub drops: Vec<f64>,
    #[props(default)]
    pub spans: Vec<Span>,
    /// Single moments, a bass swap say.
    #[props(default)]
    pub marks: Vec<f64>,
    #[props(default)]
    pub small: bool,
}

#[component]
pub fn Waveform(props: WaveProps) -> Element {
    let state = use_context::<State>();
    // Follows the file: a waveform handed another one reads that instead of
    // keeping the first.
    let path = props.path.clone();
    let peaks = use_resource(use_reactive!(|path| async move { work::blocking(move || work::peaks(&path)).await }));
    let url = crate::server::url(&props.path);
    player::title(&url, &props.title);

    let seconds = props.seconds.max(0.001);
    let x = |t: f64| t / seconds * PEAKS as f64;
    let lines = |times: &[f64]| times.iter().map(|&t| x(t)).collect::<Vec<f64>>();
    let (downbeats, phrases, sections, drops, marks) =
        (lines(&props.downbeats), lines(&props.phrases), lines(&props.sections), lines(&props.drops), lines(&props.marks));
    let spans: Vec<(f64, f64, String)> =
        props.spans.iter().map(|s| (x(s.from), (x(s.to) - x(s.from)).max(1.0), format!("span {}", s.kind))).collect();

    let player = state.player;
    let here = (player.url)() == url;
    let playhead = here.then(|| ((player.position)() / seconds * 100.0).clamp(0.0, 100.0));

    rsx! {
        div {
            class: if props.small { "wave small" } else { "wave" },
            "data-url": "{url}",
            "data-seconds": "{props.seconds}",
            title: "Click to play from here",
            svg {
                view_box: "0 0 {PEAKS} 100",
                preserve_aspect_ratio: "none",
                for (left, width, class) in spans {
                    rect { class: "{class}", x: "{left}", y: "0", width: "{width}", height: "100" }
                }
                for at in downbeats {
                    line { class: "downbeat", x1: "{at}", x2: "{at}", y1: "0", y2: "100" }
                }
                match &*peaks.read() {
                    Some(Ok(peaks)) => rsx! { path { class: "peaks", d: "{outline(peaks)}" } },
                    _ => rsx! {},
                }
                for at in phrases {
                    line { class: "phrase", x1: "{at}", x2: "{at}", y1: "0", y2: "100" }
                }
                for at in sections {
                    line { class: "section", x1: "{at}", x2: "{at}", y1: "0", y2: "100" }
                }
                for at in drops {
                    line { class: "drop", x1: "{at}", x2: "{at}", y1: "0", y2: "100" }
                }
                for at in marks {
                    line { class: "mark", x1: "{at}", x2: "{at}", y1: "0", y2: "100" }
                }
            }
            match &*peaks.read() {
                None => rsx! { div { class: "loading", "reading the audio…" } },
                Some(Err(err)) => rsx! { div { class: "loading error", "{err:#}" } },
                Some(Ok(_)) => rsx! {},
            }
            if let Some(left) = playhead {
                div { class: "playhead", style: "left: {left}%" }
            }
        }
    }
}

/// One vertical stroke a peak, mirrored about the middle.
fn outline(peaks: &[f32]) -> String {
    let mut d = String::with_capacity(peaks.len() * 16);
    for (i, p) in peaks.iter().enumerate() {
        let half = (p * 48.0).max(0.4);
        d.push_str(&format!("M{i}.5 {:.1}V{:.1}", 50.0 - half, 50.0 + half));
    }
    d
}

#[component]
pub fn Legend() -> Element {
    rsx! {
        div { class: "legend",
            span { i { style: "background: #ffffff40" } "bars" }
            span { i { style: "background: var(--accent)" } "8-bar phrases" }
            span { i { style: "background: var(--warm)" } "sections" }
            span { i { style: "background: var(--danger)" } "drops" }
            span { i { style: "background: var(--good)" } "transition" }
        }
    }
}
