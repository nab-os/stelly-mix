//! The sets kept: what has been planned and is worth keeping, and what to
//! do with one.
//!
//! A set is planned in the Set tab and lands here once saved. From here it
//! is opened, renamed, copied, exported to whatever it will be played on,
//! or forgotten.

use super::{icons, score_class, Lib, State};
use crate::work;
use dioxus::prelude::*;
use stelly_mix::export::{self, Target};
use stelly_mix::sets::Set;

/// How the list is ordered.
#[derive(Clone, Copy, PartialEq)]
enum By {
    Made,
    Name,
    Length,
    Score,
}

impl By {
    const ALL: [(By, &'static str); 4] = [(By::Made, "made"), (By::Name, "name"), (By::Length, "length"), (By::Score, "mixing")];
}

#[component]
pub fn SetsView() -> Element {
    let state = use_context::<State>();
    let mut kept: Signal<Vec<Set>> = use_signal(Vec::new);
    let mut failed: Signal<Option<String>> = use_signal(|| None);
    let mut by = use_signal(|| By::Made);
    let mut filter = use_signal(String::new);
    let mut open: Signal<Option<String>> = use_signal(|| None);

    // Read them once the view is mounted, and again whenever something here
    // changes one.
    let reload = move || {
        spawn(async move {
            match work::blocking(work::kept_sets).await {
                Ok(found) => {
                    kept.set(found);
                    failed.set(None);
                }
                Err(err) => failed.set(Some(format!("{err:#}"))),
            }
        });
    };
    use_hook(reload);

    if let Some(id) = open() {
        let found = kept.read().iter().find(|set| set.id == id).cloned();
        if let Some(set) = found {
            return rsx! {
                SetDetail {
                    set: set,
                    on_back: move |_| open.set(None),
                    on_changed: move |_| reload(),
                }
            };
        }
        open.set(None);
    }

    let needle = filter().trim().to_lowercase();
    let mut shown: Vec<Set> = kept
        .read()
        .iter()
        .filter(|set| needle.is_empty() || needle.split_whitespace().all(|word| set.name.to_lowercase().contains(word)))
        .cloned()
        .collect();
    match by() {
        By::Made => shown.sort_by_key(|set| std::cmp::Reverse(set.modified)),
        By::Name => shown.sort_by_key(|set| set.name.to_lowercase()),
        By::Length => shown.sort_by(|a, b| b.seconds.total_cmp(&a.seconds)),
        By::Score => shown.sort_by(|a, b| b.mean_score.total_cmp(&a.mean_score)),
    }

    rsx! {
        div { class: "view",
            div { class: "row-flex",
                input {
                    class: "field-filter",
                    placeholder: "filter by name",
                    value: "{filter}",
                    oninput: move |event| filter.set(event.value()),
                }
                span { class: "spacer" }
                span { class: "eyebrow", "sort" }
                div { class: "segmented",
                    for (which, label) in By::ALL {
                        button {
                            class: if by() == which { "active" } else { "" },
                            onclick: move |_| by.set(which),
                            "{label}"
                        }
                    }
                }
            }

            if let Some(reason) = failed() {
                p { class: "error selectable", "Could not read the sets: {reason}" }
            }

            if kept.read().is_empty() && failed.read().is_none() {
                div { class: "empty",
                    {icons::sets()}
                    p { "No sets yet." }
                    p { class: "small", "Plan one in the Set tab, then keep it." }
                }
            } else if shown.is_empty() {
                div { class: "empty",
                    p { "Nothing matches. Every word has to appear in the name." }
                }
            } else {
                div { class: "list",
                    for set in shown {
                        SetRow {
                            key: "{set.id}",
                            set: set.clone(),
                            on_open: move |id: String| open.set(Some(id)),
                            on_changed: move |_| reload(),
                        }
                    }
                }
            }
            if !state.library.read().is_some() {
                p { class: "muted small", "The library is still loading; a set cannot be opened until it is." }
            }
        }
    }
}

/// One set in the list, and the menu of what can be done to it.
#[component]
fn SetRow(set: Set, on_open: EventHandler<String>, on_changed: EventHandler<()>) -> Element {
    let state = use_context::<State>();
    let mut renaming = use_signal(|| false);
    let mut name = use_signal(|| set.name.clone());
    let mut arming = use_signal(|| false);

    let id = set.id.clone();
    // Takes the set rather than capturing it, so the closure stays Copy and
    // both the blur and the Enter key can call it.
    let commit = move |mut set: Set, wanted: String| {
        // A copy of the signal, so the closure needs no `&mut` of its own
        // and both handlers can call it.
        let mut renaming = renaming;
        renaming.set(false);
        if wanted.is_empty() || wanted == set.name {
            return;
        }
        set.rename(&wanted);
        spawn(async move {
            match work::blocking(move || work::put_set(&set)).await {
                Ok(()) => on_changed.call(()),
                Err(err) => state.fail(err),
            }
        });
    };

    rsx! {
        div { class: "item set-row",
            span { class: "{score_class(set.mean_score)}", "{set.mean_score:.2}" }
            div { class: "set-name",
                if renaming() {
                    input {
                        value: "{name}",
                        autofocus: true,
                        onblur: {
                            let set = set.clone();
                            move |_| commit(set.clone(), name().trim().to_string())
                        },
                        onkeydown: {
                            let set = set.clone();
                            move |event: Event<KeyboardData>| match event.key() {
                                Key::Enter => commit(set.clone(), name().trim().to_string()),
                                Key::Escape => renaming.set(false),
                                _ => {}
                            }
                        },
                        oninput: move |event| name.set(event.value()),
                    }
                } else {
                    div {
                        class: "name ellipsis",
                        title: "{set.name}",
                        onclick: {
                            let id = id.clone();
                            move |_| on_open.call(id.clone())
                        },
                        "{set.name}"
                    }
                    div { class: "detail",
                        "{set.tracks.len()} tracks · {set.clock()} · transitions averaging {set.mean_score:.2}"
                    }
                }
            }
            div { class: "actions on-hover",
                button {
                    class: "chip",
                    title: "open this set",
                    onclick: {
                        let id = id.clone();
                        move |_| on_open.call(id.clone())
                    },
                    "open"
                }
                button { class: "icon-btn", title: "rename it", onclick: move |_| renaming.set(true), {icons::rename()} }
                button {
                    class: "icon-btn",
                    title: "make a copy to change without losing this one",
                    onclick: {
                        let set = set.clone();
                        move |_| {
                            let copy = set.duplicate(&format!("{}, again", set.name));
                            spawn(async move {
                                match work::blocking(move || work::put_set(&copy)).await {
                                    Ok(()) => on_changed.call(()),
                                    Err(err) => state.fail(err),
                                }
                            });
                        }
                    },
                    {icons::duplicate()}
                }
                button {
                    class: if arming() { "chip danger armed" } else { "chip danger" },
                    title: "forget this set",
                    onclick: {
                        let id = id.clone();
                        move |_| {
                            if !arming() {
                                arming.set(true);
                                return;
                            }
                            let id = id.clone();
                            spawn(async move {
                                match work::blocking(move || work::forget_set(&id)).await {
                                    Ok(_) => on_changed.call(()),
                                    Err(err) => state.fail(err),
                                }
                            });
                        }
                    },
                    if arming() { "sure?" } else { "forget" }
                }
            }
        }
    }
}

/// One set opened: what is in it, and handing it over.
#[component]
fn SetDetail(set: Set, on_back: EventHandler<()>, on_changed: EventHandler<()>) -> Element {
    let state = use_context::<State>();
    let live = use_signal(|| set.clone());
    let mut swapping: Signal<Option<usize>> = use_signal(|| None);
    let mut offered: Signal<Vec<(String, f64)>> = use_signal(Vec::new);
    let refused: Signal<Option<String>> = use_signal(|| None);
    let mut target = use_signal(|| Target::Rekordbox);
    let mut into = use_signal(|| {
        std::env::var("HOME").map(|home| format!("{home}/Music/stelly-mix")).unwrap_or_else(|_| ".".into())
    });
    let mut exporting = use_signal(|| false);
    let mut done: Signal<Option<String>> = use_signal(|| None);

    let Some(library) = state.library.read().clone() else {
        return rsx! {
            div { class: "view",
                button { class: "link", onclick: move |_| on_back.call(()), "← all sets" }
                p { class: "muted", "loading the library…" }
            }
        };
    };
    let set = live();
    let resolved = set.resolve(&library.0.tracks);

    // Every edit is the same shape: change it, re-cut the mixes around the
    // change, keep it. A refusal leaves what is on screen alone and says so.
    let edit = move |library: Lib, change: EditKind| {
        let (mut live, mut refused, mut swapping) = (live, refused, swapping);
        let current = live();
        spawn(async move {
            let done = work::blocking(move || {
                work::edit_set(&library.0, current, move |set, tracks, stretch, scale| match change {
                    EditKind::Up(at) => set.reorder(at, at.saturating_sub(1), tracks, stretch, scale),
                    EditKind::Down(at) => set.reorder(at, at + 1, tracks, stretch, scale),
                    EditKind::Remove(at) => set.remove(at, tracks, stretch, scale),
                    EditKind::Swap(at, ref key) => set.replace(at, key, tracks, stretch, scale),
                })
            })
            .await;
            match done {
                Ok(kept) => {
                    live.set(kept);
                    refused.set(None);
                    swapping.set(None);
                    on_changed.call(());
                }
                Err(err) => refused.set(Some(format!("{err:#}"))),
            }
        });
    };

    let export = {
        let set = set.clone();
        let library = library.clone();
        move |_| {
            if exporting() {
                return;
            }
            exporting.set(true);
            done.set(None);
            let (set, library, folder, chosen) = (set.clone(), library.clone(), into(), target());
            spawn(async move {
                let options = export::Options { target: chosen, tracks: true, reference: true };
                let written = work::blocking(move || {
                    work::export_set(&set, &library.0, std::path::Path::new(&folder), &options, &|line| {
                        eprintln!("{line}");
                    })
                })
                .await;
                exporting.set(false);
                match written {
                    Ok(written) => done.set(Some(format!(
                        "{} · {} files, {:.1} GB",
                        written.dir.display(),
                        written.files.len(),
                        written.bytes as f64 / 1e9
                    ))),
                    Err(err) => state.fail(err),
                }
            });
        }
    };

    rsx! {
        div { class: "view",
            div { class: "row-flex",
                button { class: "link", onclick: move |_| on_back.call(()), "← all sets" }
            }
            div {
                div { class: "title", "{set.name}" }
                div { class: "subtitle",
                    "{set.tracks.len()} tracks · {set.clock()} · transitions averaging {set.mean_score:.2}"
                }
            }

            if !resolved.missing.is_empty() {
                p { class: "warning",
                    "{resolved.missing.len()} of its tracks have left the library, and are left out."
                }
            }

            section { class: "panel",
                h3 { {icons::export()} "Hand it over" }
                div { class: "form",
                    div { class: "field",
                        label { "To" }
                        select {
                            value: "{target().as_str()}",
                            onchange: move |event| {
                                if let Ok(chosen) = event.value().parse::<Target>() {
                                    target.set(chosen);
                                }
                            },
                            for option in Target::ALL {
                                option { value: "{option.as_str()}", "{option.title()}" }
                            }
                        }
                    }
                    div { class: "field wide",
                        label { "Into" }
                        input { value: "{into}", oninput: move |event| into.set(event.value()) }
                    }
                    button { class: "primary", disabled: exporting(), onclick: export,
                        if exporting() { "exporting…" } else { "Export" }
                    }
                }
                if let Some(caveat) = target().caveat() {
                    p { class: "muted small", "{caveat}" }
                }
                if let Some(where_to) = done() {
                    p { class: "notice small selectable", "Written to {where_to}" }
                }
            }

            section { class: "panel",
                h3 {
                    {icons::set()}
                    "The tracklist"
                    span { class: "spacer" }
                    span { class: "muted small hint", "reorder, swap or take one out; the mixes are cut again" }
                }
                if let Some(why) = refused() {
                    p { class: "error small selectable", "{why}" }
                }
                ol { class: "list",
                    for (k, &at) in resolved.plan.tracks.iter().enumerate() {
                        {
                            let track = &library.0.tracks[at];
                            let step = resolved.plan.transitions.get(k);
                            let last = k + 1 == resolved.plan.tracks.len();
                            let library = library.clone();
                            rsx! {
                                li { class: "item",
                                    div {
                                        div { class: "name ellipsis", "{track.name()}" }
                                        div { class: "detail row-flex",
                                            {super::vocals_badge(track)}
                                            span { "{track.bpm().unwrap_or(0.0):.1} BPM" }
                                            if let Some(key) = track.analysis.key.as_ref() { span { "· {key.camelot}" } }
                                            if let Some(step) = step {
                                                span { "· out at {work::clock(track.segments[step.out].start)}" }
                                            }
                                        }
                                    }
                                    div { class: "actions",
                                        span { class: "on-hover row-flex",
                                            button {
                                                class: "icon-btn",
                                                title: "move it earlier",
                                                disabled: k == 0,
                                                onclick: { let library = library.clone(); move |_| edit(library.clone(), EditKind::Up(k)) },
                                                {icons::up()}
                                            }
                                            button {
                                                class: "icon-btn",
                                                title: "move it later",
                                                disabled: last,
                                                onclick: { let library = library.clone(); move |_| edit(library.clone(), EditKind::Down(k)) },
                                                {icons::down()}
                                            }
                                            button {
                                                class: "icon-btn",
                                                title: "put another track in this slot",
                                                onclick: {
                                                    let library = library.clone();
                                                    move |_| {
                                                        if swapping() == Some(k) {
                                                            swapping.set(None);
                                                            return;
                                                        }
                                                        swapping.set(Some(k));
                                                        offered.set(Vec::new());
                                                        let (library, current) = (library.clone(), live());
                                                        spawn(async move {
                                                            let found = work::blocking(move || {
                                                                Ok(work::alternatives_for(&library.0, &current, k, 6))
                                                            })
                                                            .await
                                                            .unwrap_or_default();
                                                            offered.set(found);
                                                        });
                                                    }
                                                },
                                                {icons::swap()}
                                            }
                                            button {
                                                class: "chip",
                                                title: "keep the set up to here and plan the rest again",
                                                disabled: last,
                                                onclick: {
                                                    let library = library.clone();
                                                    move |_| {
                                                        let (mut live, mut refused, current) = (live, refused, live());
                                                        let library = library.clone();
                                                        refused.set(Some("planning the rest again…".into()));
                                                        spawn(async move {
                                                            let done = work::blocking(move || work::replan_set(&library.0, current, k)).await;
                                                            match done {
                                                                Ok(kept) => { live.set(kept); refused.set(None); on_changed.call(()); }
                                                                Err(err) => refused.set(Some(format!("{err:#}"))),
                                                            }
                                                        });
                                                    }
                                                },
                                                "from here"
                                            }
                                            button {
                                                class: "icon-btn",
                                                title: "take it out of the set",
                                                onclick: { let library = library.clone(); move |_| edit(library.clone(), EditKind::Remove(k)) },
                                                {icons::remove()}
                                            }
                                        }
                                        if let Some(step) = step {
                                            span { class: "{score_class(step.score)}", "{step.score:.2}" }
                                        } else {
                                            span { class: "muted small", "end" }
                                        }
                                    }
                                }
                                if swapping() == Some(k) {
                                    li { class: "swap-panel",
                                        if offered.read().is_empty() {
                                            span { class: "muted small", "looking for what else fits here…" }
                                        } else {
                                            span { class: "muted small", "instead of this, mixing in and out:" }
                                            for (key, score) in offered() {
                                                button {
                                                    class: "chip",
                                                    title: "put it here",
                                                    onclick: {
                                                        let (library, key) = (library.clone(), key.clone());
                                                        move |_| edit(library.clone(), EditKind::Swap(k, key.clone()))
                                                    },
                                                    "{name_of(&library, &key)} · {score:.2}"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// What an edit does to a set.
#[derive(Clone)]
enum EditKind {
    Up(usize),
    Down(usize),
    Remove(usize),
    Swap(usize, String),
}

/// A track's name from its origin key, for the swap menu.
fn name_of(library: &Lib, key: &str) -> String {
    library
        .0
        .tracks
        .iter()
        .find(|track| track.origin.as_ref().is_some_and(|origin| origin.key() == key))
        .map(|track| track.name())
        .unwrap_or_else(|| key.to_string())
}
