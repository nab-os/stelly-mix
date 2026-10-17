//! One track: what the analysis found, over its waveform, and where to go
//! from it, each way there one click from being heard.

use super::waveform::{Legend, Span, Waveform};
use super::{player, score_class, Lib, State};
use crate::work::{self, clock, Clip};
use dioxus::prelude::*;
use stelly_mix::render::Settings;
use stelly_mix::transitions::Transition;

/// Ways on listed.
const LISTED: usize = 30;

#[component]
pub fn TrackView() -> Element {
    let state = use_context::<State>();
    let Some(lib) = (state.library)() else {
        return rsx! { div { class: "view", div { class: "empty", "Reading the library…" } } };
    };
    let Some(path) = (state.selected)() else {
        return rsx! {
            div { class: "view",
                div { class: "empty", "Pick a track on the left to see its beats and phrases, and where to go from it." }
            }
        };
    };
    let Some(index) = lib.0.index_of(&path) else {
        return rsx! { div { class: "view", div { class: "empty", "{path.display()} is not in the library any more." } } };
    };
    // A list of one, keyed by the track and the library load: a key on a lone
    // component is not compared, one in a list is, so a new track or a
    // reloaded library starts the panel afresh rather than keeping hooks
    // made for the last one.
    let key = format!("{}#{:p}", path.display(), std::sync::Arc::as_ptr(&lib.0));
    rsx! {
        for key in [key] {
            TrackPanel { key: "{key}", lib: lib.clone(), index }
        }
    }
}

#[component]
fn TrackPanel(lib: Lib, index: usize) -> Element {
    let state = use_context::<State>();
    let mut stretch = use_signal(|| 6.0f64);
    let mut keylock = use_signal(|| true);
    let mut bass_swap = use_signal(|| true);
    let mut chosen = use_signal(|| None::<usize>);
    let mut clip = use_signal(|| None::<Clip>);
    let mut rendering = use_signal(|| None::<usize>);

    // The audio, fetched again first if the file it was analysed from has gone.
    let audio = use_resource({
        let lib = lib.clone();
        move || {
            let lib = lib.clone();
            async move { work::blocking(move || work::audio(&lib.0.tracks[index])).await }
        }
    });

    let next = use_resource({
        let lib = lib.clone();
        move || {
            let (lib, stretch) = (lib.clone(), stretch());
            async move { work::blocking(move || Ok(work::next(&lib.0, index, stretch / 100.0))).await }
        }
    });

    let track = &lib.0.tracks[index];
    let analysis = &track.analysis;
    let grid = analysis.grid.as_ref();
    let steady = grid.is_some_and(|g| g.steady);
    let bpm = track.bpm().map_or("no tempo".into(), |b| format!("{b:.2} BPM"));
    let key = analysis.key.as_ref().map_or("no key".into(), |k| format!("{} · {} {}", k.camelot, k.tonic, k.scale));
    let loudness = analysis.loudness.map_or(String::new(), |l| format!("{l:.1} LUFS"));
    let ways: Vec<Transition> = match &*next.read() {
        Some(Ok(ways)) => ways.iter().take(LISTED).cloned().collect(),
        _ => Vec::new(),
    };
    let out_span = chosen().and_then(|c| ways.get(c)).map(|t| {
        let segment = &track.segments[t.out];
        Span { from: segment.start, to: segment.end, kind: "out" }
    });

    let listen = use_callback({
        let lib = lib.clone();
        move |(position, transition): (usize, Transition)| {
            chosen.set(Some(position));
            rendering.set(Some(position));
            let lib = lib.clone();
            let settings = Settings { keylock: keylock(), bass_swap: bass_swap() };
            spawn(async move {
                let made = work::blocking(move || {
                    let (playing, next) = (&lib.0.tracks[index], &lib.0.tracks[transition.to]);
                    work::render_transition(playing, next, &transition, &settings)
                })
                .await;
                rendering.set(None);
                match made {
                    Ok(made) => {
                        let url = crate::server::url(&made.path);
                        player::title(&url, &made.title);
                        // From a little before the transition, to hear it come.
                        let start = made.marks.first().map_or(0.0, |m| (m.at - 8.0).max(0.0));
                        player::play(&url, start);
                        clip.set(Some(made));
                    }
                    Err(err) => state.fail(err),
                }
            });
        }
    });

    rsx! {
        div { class: "view",
            div {
                div { class: "title", "{track.name()}" }
                div { class: "muted small ellipsis", "{track.path.display()}" }
            }
            div { class: "facts",
                span { class: "fact", b { "{bpm}" } }
                span { class: if steady { "fact good" } else { "fact warm" },
                    if steady { "steady grid" } else { "drifting tempo" }
                }
                span { class: "fact", "{key}" }
                if !loudness.is_empty() {
                    span { class: "fact", "{loudness}" }
                }
                span { class: "fact", "{clock(analysis.seconds)}" }
                span { class: "fact", "{analysis.phrases.len()} phrases" }
                span { class: "fact", "downbeat confidence {analysis.downbeat_confidence:.2}" }
            }
            match &*audio.read() {
                Some(Ok(file)) => rsx! {
                    Waveform {
                        key: "{file.display()}",
                        path: file.clone(),
                        seconds: analysis.seconds,
                        title: track.name(),
                        downbeats: analysis.downbeats.clone(),
                        phrases: analysis.phrases.clone(),
                        sections: analysis.sections.clone(),
                        spans: out_span.into_iter().collect::<Vec<_>>(),
                    }
                    div { class: "row-flex",
                        Legend {}
                        span { class: "spacer" }
                        button {
                            onclick: {
                                let url = crate::server::url(file);
                                move |_| player::play(&url, 0.0)
                            },
                            "▶ Play from the top"
                        }
                    }
                },
                Some(Err(err)) => rsx! { div { class: "wave", div { class: "loading error", "{err:#}" } } },
                None => rsx! { div { class: "wave", div { class: "loading", "fetching the audio…" } } },
            }

            if let Some(made) = clip() {
                div { class: "panel",
                    h3 { "Listening" }
                    div { class: "row-flex",
                        span { class: "name", "{made.title}" }
                        span { class: "spacer" }
                        span { class: "muted small", "{clock(made.seconds)}" }
                    }
                    Waveform {
                        key: "{made.path.display()}",
                        path: made.path.clone(),
                        seconds: made.seconds,
                        title: made.title.clone(),
                        spans: made.marks.iter().filter_map(|m| m.until.map(|to| Span { from: m.at, to, kind: "into" })).collect::<Vec<_>>(),
                        marks: made.marks.iter().filter(|m| m.until.is_none()).map(|m| m.at).collect::<Vec<_>>(),
                        small: true,
                    }
                }
            }

            div { class: "panel",
                div { class: "row-flex",
                    h3 { class: "spacer", "Where to go next" }
                    label { class: "muted small", "±" }
                    input {
                        r#type: "number",
                        min: "1",
                        max: "12",
                        step: "0.5",
                        style: "width: 64px",
                        value: "{stretch}",
                        onchange: move |e| {
                            if let Ok(value) = e.value().parse::<f64>() {
                                stretch.set(value.clamp(0.5, 20.0));
                            }
                        },
                    }
                    label { class: "muted small", "% tempo" }
                    label { class: "muted small row-flex",
                        input { r#type: "checkbox", checked: keylock(), onchange: move |e| keylock.set(e.checked()) }
                        "keylock"
                    }
                    label { class: "muted small row-flex",
                        input { r#type: "checkbox", checked: bass_swap(), onchange: move |e| bass_swap.set(e.checked()) }
                        "bass swap"
                    }
                }
                match &*next.read() {
                    None => rsx! { div { class: "muted", "Weighing the library…" } },
                    Some(Err(err)) => rsx! { div { class: "error", "{err:#}" } },
                    Some(Ok(all)) if all.is_empty() => rsx! {
                        div { class: "muted", "Nothing in the library within ±{stretch}% of this tempo." }
                    },
                    Some(Ok(all)) => rsx! {
                        div { class: "muted small", "{all.len()} tracks within ±{stretch}%; the best {ways.len()}:" }
                    },
                }
                div { class: "list",
                    for (position, way) in ways.iter().cloned().enumerate() {
                        Way {
                            key: "{way.to}",
                            lib: lib.clone(),
                            playing: index,
                            way: way.clone(),
                            chosen: chosen() == Some(position),
                            rendering: rendering() == Some(position),
                            busy: rendering().is_some(),
                            on_listen: move |_| listen.call((position, way.clone())),
                            on_choose: move |_| chosen.set(Some(position)),
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Way(
    lib: Lib,
    playing: usize,
    way: Transition,
    chosen: bool,
    rendering: bool,
    busy: bool,
    on_listen: EventHandler<()>,
    on_choose: EventHandler<()>,
) -> Element {
    let state = use_context::<State>();
    let (from, to) = (&lib.0.tracks[playing], &lib.0.tracks[way.to]);
    // A way worked out for another track has nothing to show here.
    let (Some(out), Some(into)) = (from.segments.get(way.out), to.segments.get(way.into)) else {
        return rsx! {};
    };
    let key = |t: &stelly_mix::library::Track| t.analysis.key.as_ref().map_or("?".into(), |k| k.camelot.clone());
    let steps = way.key_steps.map_or(String::new(), |s| format!(" ({s} step{})", if s == 1 { "" } else { "s" }));
    let energy = way.energy.map_or(String::new(), |e| format!(" · {e:+.1} LU"));
    let path = to.path.clone();
    rsx! {
        div {
            class: if chosen { "item chosen" } else { "item" },
            onmouseenter: move |_| on_choose.call(()),
            span { class: score_class(way.score), "{way.score:.2}" }
            div {
                div { class: "name ellipsis", "{to.name()}" }
                div { class: "detail",
                    "out at {clock(out.start)} → in at {clock(into.start)} · {(way.ratio - 1.0) * 100.0:+.1}% · {key(from)} → {key(to)}{steps} · sound {way.sound:.2}{energy}"
                }
            }
            div { class: "actions",
                button {
                    class: "primary",
                    disabled: busy,
                    onclick: move |_| on_listen.call(()),
                    if rendering { "Mixing…" } else { "Listen" }
                }
                button { onclick: move |_| state.open(path.clone()), "Open" }
            }
        }
    }
}
