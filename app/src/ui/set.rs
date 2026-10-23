//! A set: planned from the track shown, or from the library's best match for
//! a description, then rendered whole and played.

use super::waveform::{Span, Waveform};
use super::{player, score_class, Lib, State};
use crate::work::{self, clock, Clip};
use dioxus::prelude::*;
use stelly_mix::set::{self, Plan};

#[component]
pub fn SetView() -> Element {
    let state = use_context::<State>();
    let Some(lib) = (state.library)() else {
        return rsx! { div { class: "view", div { class: "empty", "Reading the library…" } } };
    };
    // Afresh for each library load: a plan holds places in the one it was made in.
    let key = format!("{:p}", std::sync::Arc::as_ptr(&lib.0));
    rsx! {
        for key in [key] {
            SetPanel { key: "{key}", lib: lib.clone() }
        }
    }
}

#[component]
fn SetPanel(lib: Lib) -> Element {
    let state = use_context::<State>();
    let mut like = use_signal(String::new);
    let mut minutes = use_signal(|| 30.0f64);
    let mut stretch = use_signal(|| 6.0f64);
    let mut beam = use_signal(|| 8usize);
    let mut floor = use_signal(|| set::MIN_TRANSITION);
    let mut plan = use_signal(|| None::<Plan>);
    let mut clip = use_signal(|| None::<Clip>);
    let mut busy = use_signal(|| None::<&'static str>);
    let mut name = use_signal(String::new);
    let kept = use_signal(|| None::<String>);

    let first = (state.selected)().and_then(|p| lib.0.index_of(&p));
    let first_name = first.map(|i| lib.0.tracks[i].name());
    let can_plan = busy().is_none() && (first.is_some() || !like().trim().is_empty());

    // Takes the library rather than holding it, so it stays Copy and both
    // the button and the Enter key can call it.
    let save = move |lib: Lib| {
        let (mut busy, mut name, mut kept) = (busy, name, kept);
        {
            let Some(planned) = plan() else { return };
            let wanted = name().trim().to_string();
            if wanted.is_empty() {
                return;
            }
            let terms = stelly_mix::sets::Planned {
                minutes: minutes(),
                max_stretch: stretch() / 100.0,
                beam: beam(),
                min_transition: floor(),
                like: Some(like().trim().to_string()).filter(|text| !text.is_empty()),
            };
            busy.set(Some("keeping it"));
            spawn(async move {
                let saved = work::blocking(move || work::save_set(&lib.0, &planned, &wanted, terms)).await;
                busy.set(None);
                match saved {
                    Ok(set) => {
                        kept.set(Some(set.name.clone()));
                        name.set(String::new());
                    }
                    Err(err) => state.fail(err),
                }
            });
        }
    };

    let make_plan = {
        let lib = lib.clone();
        move |_| {
            let lib = lib.clone();
            let description = like().trim().to_string();
            let (minutes, stretch, beam, floor) = (minutes(), stretch(), beam(), floor());
            busy.set(Some("Planning…"));
            clip.set(None);
            spawn(async move {
                let planned = work::blocking(move || {
                    let liked = if description.is_empty() { None } else { Some(work::like(&lib.0, &description)?) };
                    // The track shown, or else the best match that keeps a steady tempo.
                    let start = match (first, &liked) {
                        (Some(first), _) => first,
                        (None, Some(liked)) => (0..lib.0.tracks.len())
                            .filter(|&i| lib.0.tracks[i].analysis.grid.as_ref().is_some_and(|g| g.steady))
                            .max_by(|&a, &b| liked[a].total_cmp(&liked[b]))
                            .ok_or_else(|| anyhow::anyhow!("no track in the library keeps a steady tempo"))?,
                        (None, None) => anyhow::bail!("pick a first track, or describe the set"),
                    };
                    let options = set::Options {
                        seconds: minutes * 60.0,
                        max_stretch: stretch / 100.0,
                        beam: beam.max(1),
                        like: liked,
                        min_transition: floor,
                        ..Default::default()
                    };
                    work::plan(&lib.0, start, &options)
                })
                .await;
                busy.set(None);
                match planned {
                    Ok(planned) => plan.set(Some(planned)),
                    Err(err) => state.fail(err),
                }
            });
        }
    };

    let render = {
        let lib = lib.clone();
        move |_| {
            let Some(planned) = plan() else { return };
            let lib = lib.clone();
            busy.set(Some("Rendering the set…"));
            spawn(async move {
                let made = work::blocking(move || work::render_set(&lib.0, &planned)).await;
                busy.set(None);
                match made {
                    Ok(made) => {
                        let url = crate::server::url(&made.path);
                        player::title(&url, &made.title);
                        player::play(&url, 0.0);
                        clip.set(Some(made));
                    }
                    Err(err) => state.fail(err),
                }
            });
        }
    };

    let master = plan().and_then(|p| lib.0.tracks[p.tracks[0]].bpm());

    rsx! {
        div { class: "view",
            div { class: "title", "Plan a set" }
            div { class: "panel",
                div { class: "form",
                    div { class: "field",
                        label { "From" }
                        div { class: "ellipsis", style: "max-width: 320px; padding: 5px 0",
                            match &first_name {
                                Some(name) => rsx! { "{name}" },
                                None => rsx! { span { class: "muted", "the best match for the description" } },
                            }
                        }
                    }
                    div { class: "field wide",
                        label { "Sounding like (optional)" }
                        input {
                            placeholder: "e.g. dark rolling techno",
                            value: "{like}",
                            oninput: move |e| like.set(e.value()),
                        }
                    }
                    div { class: "field",
                        label { "Minutes" }
                        input { r#type: "number", min: "2", step: "1", value: "{minutes}",
                            onchange: move |e| if let Ok(v) = e.value().parse::<f64>() { minutes.set(v.max(1.0)) },
                        }
                    }
                    div { class: "field",
                        label { "± tempo %" }
                        input { r#type: "number", min: "0.5", step: "0.5", value: "{stretch}",
                            onchange: move |e| if let Ok(v) = e.value().parse::<f64>() { stretch.set(v.clamp(0.5, 20.0)) },
                        }
                    }
                    div { class: "field",
                        label { "Beam" }
                        input { r#type: "number", min: "1", step: "1", value: "{beam}",
                            onchange: move |e| if let Ok(v) = e.value().parse::<usize>() { beam.set(v.clamp(1, 64)) },
                        }
                    }
                    div { class: "field",
                        label { "Weakest transition" }
                        input { r#type: "number", min: "0", max: "1", step: "0.05", value: "{floor}",
                            onchange: move |e| if let Ok(v) = e.value().parse::<f64>() { floor.set(v.clamp(0.0, 1.0)) },
                        }
                    }
                    button { class: "primary", disabled: !can_plan, onclick: make_plan, "Plan" }
                    button { disabled: plan().is_none() || busy().is_some(), onclick: render, "Render and play" }
                }
                if plan().is_some() {
                    div { class: "form",
                        div { class: "field wide",
                            label { "Keep it as" }
                            input {
                                placeholder: "a name you will recognise",
                                value: "{name}",
                                oninput: move |event| name.set(event.value()),
                                onkeydown: {
                                    let lib = lib.clone();
                                    move |event: Event<KeyboardData>| {
                                        if event.key() == Key::Enter { save(lib.clone()); }
                                    }
                                },
                            }
                        }
                        button {
                            disabled: name().trim().is_empty() || busy().is_some(),
                            onclick: {
                                let lib = lib.clone();
                                move |_| save(lib.clone())
                            },
                            "Keep"
                        }
                    }
                }
                if let Some(what) = busy() {
                    div { class: "muted small", "{what}" }
                }
                if let Some(under) = kept() {
                    div { class: "notice small", "Kept as {under}. The Sets tab has it." }
                }
            }

            if let Some(made) = clip() {
                div { class: "panel",
                    h3 { "The set" }
                    Waveform {
                        key: "{made.path.display()}",
                        path: made.path.clone(),
                        seconds: made.seconds,
                        title: made.title.clone(),
                        spans: made.marks.iter().filter_map(|m| m.until.map(|to| Span { from: m.at, to, kind: "into" })).collect::<Vec<_>>(),
                        marks: made.marks.iter().map(|m| m.at).collect::<Vec<_>>(),
                    }
                }
            }

            match plan() {
                None => rsx! {
                    div { class: "empty", "A set keeps the first track's tempo, takes no track twice, and runs as long as asked." }
                },
                Some(planned) => rsx! {
                    div { class: "panel",
                        h3 {
                            "{planned.tracks.len()} tracks, {planned.transitions.len()} transitions averaging {planned.mean_score():.2}, "
                            "{clock(planned.seconds)} before the last comes in"
                        }
                        div { class: "list",
                            for (k, &i) in planned.tracks.iter().enumerate() {
                                SetRow {
                                    key: "{k}",
                                    lib: lib.clone(),
                                    track: i,
                                    master: master.unwrap_or(0.0),
                                    enters: clip().and_then(|c| c.marks.get(k).map(|m| m.at)),
                                    url: clip().map(|c| crate::server::url(&c.path)),
                                    leaves: planned.transitions.get(k).map(|t| (t.score, lib.0.tracks[i].segments[t.out].start, lib.0.tracks[t.to].segments[t.into].start)),
                                }
                            }
                        }
                    }
                },
            }
        }
    }
}

#[component]
fn SetRow(
    lib: Lib,
    track: usize,
    master: f64,
    /// Where it comes in, once the set is rendered.
    enters: Option<f64>,
    url: Option<String>,
    /// The transition out of it: its score, where it leaves, where the next comes in.
    leaves: Option<(f64, f64, f64)>,
) -> Element {
    let state = use_context::<State>();
    let t = &lib.0.tracks[track];
    let bpm = t.bpm().unwrap_or(0.0);
    let key = t.analysis.key.as_ref().map_or("?".into(), |k| k.camelot.clone());
    let path = t.path.clone();
    rsx! {
        div { class: "item",
            match leaves {
                Some((score, _, _)) => rsx! { span { class: score_class(score), "{score:.2}" } },
                None => rsx! { span { class: "muted", "end" } },
            }
            div {
                div { class: "name ellipsis", "{t.name()}" }
                div { class: "detail",
                    "{bpm:.2} BPM played at {master:.2} · {key}"
                    if let Some((_, out, into)) = leaves {
                        " · out at {clock(out)} into the next at {clock(into)}"
                    }
                }
            }
            div { class: "actions",
                if let (Some(at), Some(url)) = (enters, url) {
                    button { onclick: move |_| player::play(&url, at), "▶ {clock(at)}" }
                }
                button { onclick: move |_| state.open(path.clone()), "Open" }
            }
        }
    }
}
