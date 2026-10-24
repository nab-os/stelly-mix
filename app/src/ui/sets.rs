//! Sets: the ones you keep, and the one you are working on.
//!
//! Planning and editing used to be two tabs, which meant a set existed in
//! two places and belonged to neither. Here there is one: a list of what is
//! kept, and an editor you reach by opening one or starting a new one. The
//! planner is the top of that editor, because planning is how a set starts,
//! not a different activity.
//!
//! Nothing here writes to disk until Save. An edit changes what is on
//! screen; the badge by the name says whether that has been kept.

use super::waveform::{Span, Waveform};
use super::{icons, score_class, Lib, State};
use crate::work::Clip;
use crate::work;
use dioxus::prelude::*;
use stelly_mix::export::{self, Target};
use stelly_mix::sets::{Planned, Set};

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

/// The workbench: the set being worked on and where the next track goes.
/// Shared, because the source pane down the left is the other half of it —
/// it has to know what the set is to rank anything against it, and where
/// the insert point is to put anything anywhere.
#[derive(Clone, Copy)]
pub struct Bench {
    pub editing: Signal<Option<Editing>>,
    /// The gap a track from the source drops into. `None` is the end.
    pub insert_at: Signal<Option<usize>>,
    /// A track the source asked to put in, by origin key.
    pub wanted: Signal<Option<String>>,
}

impl Bench {
    pub fn provide() -> Self {
        use_context_provider(|| Bench {
            editing: Signal::new(None),
            insert_at: Signal::new(None),
            wanted: Signal::new(None),
        })
    }

    /// The set as it stands, for the source pane to rank against.
    pub fn set(&self) -> Option<Set> {
        self.editing.read().as_ref().map(|held| held.set.clone())
    }
}

/// The set being worked on, and the copy that is on disk, if any.
#[derive(Clone, PartialEq)]
pub struct Editing {
    set: Set,
    /// `None` for a set that has never been kept.
    saved: Option<Set>,
}

impl Editing {
    /// Whether what is on screen differs from what is kept. Compared on what
    /// a set *is* rather than the whole struct: every re-cut stamps
    /// `modified`, so comparing those would read as changed the moment
    /// anything ran at all.
    fn unsaved(&self) -> bool {
        match &self.saved {
            None => true,
            Some(saved) => saved.name != self.set.name || saved.tracks != self.set.tracks || saved.steps != self.set.steps,
        }
    }
}

/// What an edit does to a set.
#[derive(Clone, PartialEq)]
enum Edit {
    Up(usize),
    Down(usize),
    Remove(usize),
    Swap(usize, String),
    Insert(usize, String),
    Replan(usize),
}

#[component]
pub fn SetsView() -> Element {
    let mut kept: Signal<Vec<Set>> = use_signal(Vec::new);
    let mut failed: Signal<Option<String>> = use_signal(|| None);
    let mut by = use_signal(|| By::Made);
    let mut filter = use_signal(String::new);
    let bench = use_context::<Bench>();
    let mut open = bench.editing;

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

    if open.read().is_some() {
        return rsx! { Editor { open, on_kept: move |_| reload() } };
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
                input { placeholder: "filter by name", value: "{filter}", oninput: move |event| filter.set(event.value()) }
                span { class: "eyebrow", "sort" }
                div { class: "segmented",
                    for (which, label) in By::ALL {
                        button { class: if by() == which { "active" } else { "" }, onclick: move |_| by.set(which), "{label}" }
                    }
                }
                span { class: "spacer" }
                button { class: "primary", onclick: move |_| open.set(Some(Editing { set: blank(), saved: None })),
                    {icons::plus()}
                    "New set"
                }
            }

            if let Some(reason) = failed() {
                p { class: "error small selectable", "Could not read the sets: {reason}" }
            }

            if kept.read().is_empty() && failed.read().is_none() {
                div { class: "empty",
                    {icons::sets()}
                    p { "No sets yet." }
                    p { class: "small", "Start one, plan it from a track or a description, and keep it." }
                }
            } else if shown.is_empty() {
                div { class: "empty", p { "Nothing matches. Every word has to appear in the name." } }
            } else {
                div { class: "list",
                    for set in shown {
                        SetRow { key: "{set.id}", set: set.clone(), open, on_changed: move |_| reload() }
                    }
                }
            }
        }
    }
}

/// A set with nothing in it yet.
fn blank() -> Set {
    Set {
        id: String::new(),
        name: String::new(),
        version: stelly_mix::sets::VERSION,
        created: 0,
        modified: 0,
        tracks: Vec::new(),
        steps: Vec::new(),
        seconds: 0.0,
        mean_score: 0.0,
        planned: Planned::default(),
    }
}

#[component]
fn SetRow(set: Set, open: Signal<Option<Editing>>, on_changed: EventHandler<()>) -> Element {
    let state = use_context::<State>();
    let mut arming = use_signal(|| false);
    let id = set.id.clone();

    rsx! {
        div { class: "item set-row",
            span { class: "{score_class(set.mean_score)}", "{set.mean_score:.2}" }
            div {
                div {
                    class: "name ellipsis clickable",
                    onclick: {
                        let set = set.clone();
                        move |_| open.set(Some(Editing { set: set.clone(), saved: Some(set.clone()) }))
                    },
                    "{set.name}"
                }
                div { class: "detail", "{set.tracks.len()} tracks · {set.clock()} · transitions averaging {set.mean_score:.2}" }
            }
            div { class: "actions on-hover",
                button {
                    class: "secondary",
                    onclick: {
                        let set = set.clone();
                        move |_| open.set(Some(Editing { set: set.clone(), saved: Some(set.clone()) }))
                    },
                    "Open"
                }
                button {
                    class: "ghost",
                    title: "a copy to change without losing this one",
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
                    class: if arming() { "danger armed" } else { "danger" },
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
                    if arming() { "Sure?" } else { "Forget" }
                }
            }
        }
    }
}

/// The set being worked on: plan it, shape it, keep it, hand it over.
#[component]
fn Editor(open: Signal<Option<Editing>>, on_kept: EventHandler<()>) -> Element {
    let state = use_context::<State>();
    let busy: Signal<Option<String>> = use_signal(|| None);
    let failed: Signal<Option<String>> = use_signal(|| None);
    let mut picking: Signal<Option<Pick>> = use_signal(|| None);
    let mut saving_as = use_signal(|| false);
    let mut as_name = use_signal(String::new);
    // The last render of this set, to listen back to and to read the
    // transitions off. Cleared whenever the tracklist changes under it.
    let clip: Signal<Option<Clip>> = use_signal(|| None);
    // Set when leaving was asked for and there is something unkept: the
    // question is asked in the page rather than in a dialog, so the set is
    // still visible behind it while you decide.
    let mut leaving = use_signal(|| false);

    // Every edit is the same shape: change the copy on screen, re-cut the
    // mixes around the change, and leave the disk alone.
    let edit = move |(library, change): (Lib, Edit)| {
        let (mut open, mut failed, mut picking, mut busy, mut clip) = (open, failed, picking, busy, clip);
        let Some(current) = open.read().clone() else { return };
        busy.set(Some("re-cutting".into()));
        spawn(async move {
            let done = work::blocking(move || match change {
                Edit::Replan(at) => work::replan_set(&library.0, current.set.clone(), at),
                other => work::edit_set(&library.0, current.set.clone(), move |set, tracks, stretch, scale| match other {
                    Edit::Up(at) => set.reorder(at, at.saturating_sub(1), tracks, stretch, scale),
                    Edit::Down(at) => set.reorder(at, at + 1, tracks, stretch, scale),
                    Edit::Remove(at) => set.remove(at, tracks, stretch, scale),
                    Edit::Swap(at, ref key) => set.replace(at, key, tracks, stretch, scale),
                    Edit::Insert(at, ref key) => set.insert(at, key, tracks, stretch, scale),
                    Edit::Replan(_) => unreachable!("taken above"),
                }),
            })
            .await;
            busy.set(None);
            match done {
                Ok(next) => {
                    open.with_mut(|held| {
                        if let Some(held) = held {
                            held.set = next;
                        }
                    });
                    failed.set(None);
                    picking.set(None);
                    // The render was of the old tracklist.
                    clip.set(None);
                }
                Err(err) => failed.set(Some(format!("{err:#}"))),
            }
        });
    };

    let hear = move |library: Lib| {
        let (mut busy, mut failed, mut clip) = (busy, failed, clip);
        let Some(current) = open.read().clone() else { return };
        busy.set(Some("rendering the set".into()));
        spawn(async move {
            let made = work::blocking(move || {
                let plan = current.set.resolve(&library.0.tracks).plan;
                work::render_set(&library.0, &plan)
            })
            .await;
            busy.set(None);
            match made {
                Ok(made) => {
                    let url = crate::server::url(&made.path);
                    super::player::title(&url, &made.title);
                    super::player::play(&url, 0.0);
                    clip.set(Some(made));
                }
                Err(err) => failed.set(Some(format!("{err:#}"))),
            }
        });
    };

    // Whatever the source pane asked to put in, at the insert point it was
    // pointing at. Declared with the rest of the hooks: below the early
    // returns it would be skipped while the library loads.
    let mut bench = use_context::<Bench>();
    use_effect(move || {
        let Some(key) = bench.wanted.read().clone() else { return };
        let Some(library) = state.library.peek().clone() else { return };
        bench.wanted.set(None);
        let at = bench.insert_at.peek().unwrap_or(usize::MAX);
        edit((library, Edit::Insert(at, key)));
    });

    let Some(library) = state.library.read().clone() else {
        return rsx! { div { class: "view", p { class: "muted", "loading the library…" } } };
    };
    let Some(editing) = open.read().clone() else { return rsx! {} };
    let set = editing.set.clone();
    let unsaved = editing.unsaved();

    let resolved = set.resolve(&library.0.tracks);
    // The set holds the first track's tempo; every other row says how far
    // it is being stretched to get there.
    let master = resolved.plan.tracks.first().and_then(|&at| library.0.tracks[at].bpm());

    let keep = move |name: String, as_copy: bool| {
        let (mut open, mut failed, mut busy, mut saving_as) = (open, failed, busy, saving_as);
        let Some(current) = open.read().clone() else { return };
        let name = name.trim().to_string();
        if name.is_empty() {
            failed.set(Some("A set wants a name before it can be kept.".into()));
            return;
        }
        busy.set(Some("keeping it".into()));
        spawn(async move {
            let kept = work::blocking(move || {
                if as_copy || current.saved.is_none() {
                    work::save_set_as(&current.set, &name)
                } else {
                    let mut set = current.set.clone();
                    set.rename(&name);
                    work::put_set(&set)?;
                    Ok(set)
                }
            })
            .await;
            busy.set(None);
            saving_as.set(false);
            match kept {
                Ok(saved) => {
                    open.set(Some(Editing { set: saved.clone(), saved: Some(saved) }));
                    failed.set(None);
                    on_kept.call(());
                }
                Err(err) => failed.set(Some(format!("{err:#}"))),
            }
        });
    };

    rsx! {
        div { class: "view",
            div { class: "row-flex editor-head",
                button {
                    class: "link",
                    onclick: move |_| {
                        if unsaved && !set.tracks.is_empty() {
                            leaving.set(true);
                        } else {
                            open.set(None);
                        }
                    },
                    "← all sets"
                }
                input {
                    class: "set-name",
                    placeholder: "name this set",
                    value: "{set.name}",
                    oninput: move |event| {
                        let value = event.value();
                        open.with_mut(|held| {
                            if let Some(held) = held {
                                held.set.name = value;
                            }
                        });
                    },
                }
                if unsaved {
                    span { class: "badge warm", title: "not kept yet", "unsaved" }
                } else {
                    span { class: "badge good", "kept" }
                }
                span { class: "spacer" }
                if let Some(what) = busy() {
                    span { class: "row-flex", span { class: "working-dot" } span { class: "muted small", "{what}" } }
                }
                button {
                    class: "primary",
                    disabled: !unsaved || busy().is_some(),
                    onclick: {
                        let name = set.name.clone();
                        move |_| keep(name.clone(), false)
                    },
                    {icons::save()}
                    "Save"
                }
                button { class: "secondary", disabled: busy().is_some(), onclick: move |_| saving_as.set(!saving_as()), "Save as…" }
            }

            if leaving() {
                div { class: "warning row-flex",
                    span { "“{set.name}” has changes that are not kept." }
                    span { class: "spacer" }
                    button {
                        class: "primary",
                        onclick: {
                            let name = set.name.clone();
                            move |_| {
                                leaving.set(false);
                                keep(name.clone(), false);
                            }
                        },
                        "Save it"
                    }
                    button { class: "danger", onclick: move |_| { leaving.set(false); open.set(None) }, "Leave without saving" }
                    button { class: "ghost", onclick: move |_| leaving.set(false), "Stay" }
                }
            }

            if saving_as() {
                div { class: "panel inset row-flex",
                    input {
                        placeholder: "a name for the copy",
                        value: "{as_name}",
                        oninput: move |event| as_name.set(event.value()),
                        onkeydown: move |event: Event<KeyboardData>| {
                            if event.key() == Key::Enter {
                                keep(as_name(), true);
                            }
                        },
                    }
                    button { class: "primary", onclick: move |_| keep(as_name(), true), "Keep a copy" }
                    button { class: "ghost", onclick: move |_| saving_as.set(false), "Cancel" }
                }
            }

            if let Some(why) = failed() {
                p { class: "error small selectable", "{why}" }
            }
            if !resolved.missing.is_empty() {
                p { class: "warning", "{resolved.missing.len()} of its tracks have left the library, and are left out." }
            }

            Planner { open, busy, failed, library: library.clone() }

            section { class: "panel",
                h3 {
                    {icons::set()}
                    "Tracklist"
                    span { class: "spacer" }
                    span { class: "muted small hint",
                        if set.tracks.is_empty() {
                            "plan one above, or add tracks to it below"
                        } else {
                            "{set.tracks.len()} tracks · {set.clock()} · averaging {set.mean_score:.2}"
                        }
                    }
                    if !set.tracks.is_empty() {
                        button {
                            class: "secondary",
                            disabled: busy().is_some(),
                            title: "render the whole set and play it",
                            onclick: {
                                let library = library.clone();
                                move |_| hear(library.clone())
                            },
                            {icons::play()}
                            "Render & play"
                        }
                    }
                }
                div { class: "tracklist",
                    div { class: "tl-head",
                        span { class: "tl-n" }
                        span { "TRACK" }
                        span { "TEMPO" }
                        span { "KEY" }
                        span { "PLAYS" }
                        span { class: "tl-act" }
                    }
                    for (k, &at) in resolved.plan.tracks.iter().enumerate() {
                        {
                            let track = &library.0.tracks[at];
                            let step = resolved.plan.transitions.get(k);
                            let last = k + 1 == resolved.plan.tracks.len();
                            let library = library.clone();
                            let (artist, title) = match track.origin.as_ref() {
                                Some(o) => (o.item.artist.clone(), o.item.title.clone()),
                                None => (String::new(), track.name()),
                            };
                            let stretch = master.map(|m| track.bpm().map_or(0.0, |b| (m / b - 1.0) * 100.0));
                            rsx! {
                                div { class: "tl-track",
                                    span { class: "tl-n", "{k + 1}" }
                                    div { class: "tl-of",
                                        div { class: "tl-title row-flex",
                                            span { "{title}" }
                                            {super::vocals_badge(track)}
                                        }
                                        div { class: "tl-artist", "{artist}" }
                                    }
                                    span { class: "tl-tempo num",
                                        "{track.bpm().unwrap_or(0.0):.1}"
                                        if let Some(by) = stretch {
                                            span { class: "muted", " {by:+.1}%" }
                                        }
                                    }
                                    span { class: "tl-key num",
                                        {track.analysis.key.as_ref().map_or("–".to_string(), |key| key.camelot.clone())}
                                    }
                                    span { class: "tl-plays num", "{work::clock(played(track, step))}" }
                                    div { class: "tl-act on-hover",
                                        button { class: "ghost", title: "move it earlier", disabled: k == 0,
                                            onclick: { let l = library.clone(); move |_| edit((l.clone(), Edit::Up(k))) }, {icons::up()} }
                                        button { class: "ghost", title: "move it later", disabled: last,
                                            onclick: { let l = library.clone(); move |_| edit((l.clone(), Edit::Down(k))) }, {icons::down()} }
                                        button { class: "ghost",
                                            onclick: move |_| picking.set(match picking() {
                                                Some(Pick { at: a, swapping: true }) if a == k => None,
                                                _ => Some(Pick { at: k, swapping: true }),
                                            }),
                                            if picking() == Some(Pick { at: k, swapping: true }) { "Close alternatives" } else { "Swap" }
                                        }
                                        button { class: "ghost", title: "take it out", 
                                            onclick: { let l = library.clone(); move |_| edit((l.clone(), Edit::Remove(k))) }, "Remove" }
                                        button { class: "ghost", disabled: last,
                                            title: "keep the set to here and plan the rest again",
                                            onclick: { let l = library.clone(); move |_| edit((l.clone(), Edit::Replan(k))) },
                                            "Keep to here, re-plan rest"
                                        }
                                    }
                                }
                                if picking() == Some(Pick { at: k, swapping: true }) {
                                    div { class: "tl-panel",
                                        div { class: "muted small", "Instead of " b { "{title}" } ", ranked by both of its transitions" }
                                        Picker { at: k, swapping: true, open, picking, library: library.clone(), edit }
                                    }
                                }
                                // The mix itself: where it happens and how good it is.
                                if let Some(step) = step {
                                    {
                                        let next = &library.0.tracks[resolved.plan.tracks[k + 1]];
                                        let (out, into) = (track.segments[step.out].start, next.segments[step.into].start);
                                        let keys = match (track.analysis.key.as_ref(), next.analysis.key.as_ref()) {
                                            (Some(a), Some(b)) => format!("{} → {}", a.camelot, b.camelot),
                                            _ => "–".into(),
                                        };
                                        let steps = step.key_steps.map(|n| format!(" · {n}")).unwrap_or_default();
                                        let energy = step.energy.map(|lu| format!(" · {lu:+.1} LU")).unwrap_or_default();
                                        let pct = (step.ratio - 1.0) * 100.0;
                                        rsx! {
                                            div { class: "tl-mix",
                                                span { class: "tl-n" }
                                                span { class: "mix-score {score_class(step.score)}", "{step.score:.2}" }
                                                span { class: "mix-of",
                                                    "out at {work::clock(out)} → in at {work::clock(into)} · {pct:+.1}% tempo · {keys}{steps} · sound {step.sound:.2}{energy}"
                                                }
                                                div { class: "mix-act",
                                                    button { class: "ghost insert on-hover",
                                                        onclick: move |_| picking.set(match picking() {
                                                            Some(Pick { at: a, swapping: false }) if a == k + 1 => None,
                                                            _ => Some(Pick { at: k + 1, swapping: false }),
                                                        }),
                                                        "+ insert between"
                                                    }
                                                    if let Some(at) = clip().and_then(|c| c.marks.get(k).map(|m| m.at)) {
                                                        button { class: "ghost",
                                                            onclick: {
                                                                let url = clip().map(|c| crate::server::url(&c.path));
                                                                move |_| if let Some(url) = &url { super::player::play(url, (at - 8.0).max(0.0)) }
                                                            },
                                                            {icons::play()}
                                                            "hear"
                                                        }
                                                    }
                                                }
                                            }
                                            if picking() == Some(Pick { at: k + 1, swapping: false }) {
                                                div { class: "tl-panel",
                                                    Picker { at: k + 1, swapping: false, open, picking, library: library.clone(), edit }
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

            if let Some(made) = clip() {
                section { class: "panel",
                    h3 {
                        {icons::play()}
                        "The rendered set"
                        span { class: "spacer" }
                        span { class: "muted small hint", "each track's entry marked, each transition shaded" }
                    }
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

            Exporter { set: set.clone(), library: library.clone(), unsaved }
        }
    }
}

/// Which gap or slot the picker is open on.
#[derive(Clone, Copy, PartialEq)]
struct Pick {
    at: usize,
    /// Replacing what is there, rather than going in beside it.
    swapping: bool,
}

/// The line between two tracks that opens a picker.
#[component]
fn GapLine(at: usize, total: usize, picking: Signal<Option<Pick>>, open: Signal<Option<Editing>>, library: Lib, edit: EventHandler<(Lib, Edit)>) -> Element {
    let mut picking = picking;
    let mut bench = use_context::<Bench>();
    let mut over = use_signal(|| false);
    let open_here = picking() == Some(Pick { at, swapping: false });
    // The one the source pane is aiming at. The end is the default, so the
    // last gap is marked when nothing has been picked.
    let marked = bench.insert_at.read().map_or(at + 1 == total, |picked| picked == at);
    rsx! {
        li {
            class: match (open_here, over(), marked) {
                (_, true, _) => "gap over",
                (true, _, _) => "gap open",
                (_, _, true) => "gap marked",
                _ => "gap",
            },
            onclick: move |_| bench.insert_at.set(Some(at)),
            ondragover: move |event| {
                event.prevent_default();
                if !over() {
                    over.set(true);
                }
            },
            ondragleave: move |_| over.set(false),
            ondrop: {
                let library = library.clone();
                move |event| {
                    event.prevent_default();
                    over.set(false);
                    if let Some(key) = bench.wanted.read().clone() {
                        edit((library.clone(), Edit::Insert(at, key)));
                    }
                }
            },
            button {
                class: "gap-open",
                title: "add a track here",
                onclick: move |_| picking.set(if open_here { None } else { Some(Pick { at, swapping: false }) }),
                {icons::plus()}
                span { if marked { "the source puts tracks here" } else { "put tracks here" } }
            }
        }
        if open_here {
            Picker { at, swapping: false, open, picking, library: library.clone(), edit }
        }
    }
}

/// What to put in a gap or a slot: what fits there, and a search for when
/// you already know what you want.
#[component]
fn Picker(
    at: usize,
    swapping: bool,
    open: Signal<Option<Editing>>,
    picking: Signal<Option<Pick>>,
    library: Lib,
    edit: EventHandler<(Lib, Edit)>,
) -> Element {
    let mut picking = picking;
    let mut words = use_signal(String::new);
    let mut found: Signal<Vec<(String, String, Option<f64>)>> = use_signal(Vec::new);
    let mut fits: Signal<Vec<stelly_mix::sets::Fit>> = use_signal(Vec::new);

    let starting = library.clone();
    use_hook(move || {
        let lib = starting.clone();
        spawn(async move {
            let Some(current) = open.read().clone() else { return };
            let offered = work::blocking(move || {
                Ok(if swapping {
                    work::alternatives_for(&lib.0, &current.set, at, 8)
                } else {
                    work::fits_between(&lib.0, &current.set, at, 8)
                })
            })
            .await
            .unwrap_or_default();
            fits.set(offered);
        });
    });

    let searching = library.clone();
    let search = move |text: String| {
        let lib = searching.clone();
        spawn(async move {
            let Some(current) = open.read().clone() else { return };
            if text.trim().is_empty() {
                found.set(Vec::new());
                return;
            }
            let hits =
                work::blocking(move || Ok(work::search_for_gap(&lib.0, &current.set, at, &text, 12))).await.unwrap_or_default();
            found.set(hits);
        });
    };

    // Takes the library each time rather than holding it, so the closure
    // stays Copy and every pick in the list can call it.
    let chosen = move |(library, key): (Lib, String)| {
        edit((library, if swapping { Edit::Swap(at, key.clone()) } else { Edit::Insert(at, key) }));
    };
    let name_of = |key: &str| -> String {
        library
            .0
            .tracks
            .iter()
            .find(|t| t.origin.as_ref().is_some_and(|o| o.key() == key))
            .map(|t| t.name())
            .unwrap_or_else(|| key.to_string())
    };

    rsx! {
        li { class: "picker",
            div { class: "row-flex",
                input {
                    class: "wide",
                    placeholder: if swapping { "search for a track to put here" } else { "search for a track to add here" },
                    value: "{words}",
                    oninput: move |event| {
                        let text = event.value();
                        words.set(text.clone());
                        search(text);
                    },
                }
                button { class: "ghost", onclick: move |_| picking.set(None), "Cancel" }
            }
            div { class: "picks",
                if !found.read().is_empty() {
                    for (key, name, fit) in found() {
                        button { class: "pick", onclick: {
                                let (library, key) = (library.clone(), key.clone());
                                move |_| chosen((library.clone(), key.clone()))
                            },
                            span { class: "ellipsis", "{name}" }
                            if let Some(fit) = fit {
                                span { class: "{score_class(fit)}", "{fit:.2}" }
                            } else {
                                span { class: "muted small", title: "too far in tempo to mix here", "–" }
                            }
                        }
                    }
                } else if fits.read().is_empty() {
                    span { class: "muted small", "looking for what fits here…" }
                } else {
                    // Both sides, because a swap changes two mixes and which
                    // of them it spoils is the thing worth seeing.
                    div { class: "pick-head",
                        span { class: "spacer" }
                        span { "in" }
                        span { "out" }
                        span { "mean" }
                    }
                    for fit in fits() {
                        button { class: "pick", onclick: {
                                let (library, key) = (library.clone(), fit.key.clone());
                                move |_| chosen((library.clone(), key.clone()))
                            },
                            span { class: "ellipsis", "{name_of(&fit.key)}" }
                            match fit.coming_in {
                                Some(score) => rsx! { span { class: "{score_class(score)}", "{score:.2}" } },
                                None => rsx! { span { class: "muted", "–" } },
                            }
                            match fit.going_out {
                                Some(score) => rsx! { span { class: "{score_class(score)}", "{score:.2}" } },
                                None => rsx! { span { class: "muted", "–" } },
                            }
                            span { class: "{score_class(fit.mean)} mean", "{fit.mean:.2}" }
                        }
                    }
                }
            }
        }
    }
}

/// Planning, which is how a set starts and how it is filled again.
#[component]
fn Planner(open: Signal<Option<Editing>>, busy: Signal<Option<String>>, failed: Signal<Option<String>>, library: Lib) -> Element {
    let state = use_context::<State>();
    let Some(editing) = open.read().clone() else { return rsx! {} };
    let planned = editing.set.planned.clone();
    let empty = editing.set.tracks.is_empty();
    let first = (state.selected)().and_then(|p| library.0.index_of(&p));
    let first_name = first.map(|i| library.0.tracks[i].name());

    let change = move |what: fn(&mut Planned, f64), value: f64| {
        let mut open = open;
        open.with_mut(|held| {
            if let Some(held) = held {
                what(&mut held.set.planned, value);
            }
        });
    };
    let number = move |label: &'static str, value: f64, step: &'static str, what: fn(&mut Planned, f64)| {
        rsx! {
            div { class: "field",
                label { "{label}" }
                input { r#type: "number", step: "{step}", value: "{value}",
                    onchange: move |event| if let Ok(v) = event.value().parse::<f64>() { change(what, v) } }
            }
        }
    };

    let plan = move |library: Lib| {
        let (mut open, mut busy, mut failed) = (open, busy, failed);
        let Some(current) = open.read().clone() else { return };
        busy.set(Some("planning".into()));
        spawn(async move {
            let made = work::blocking(move || work::plan_into(&library.0, current.set.clone(), first)).await;
            busy.set(None);
            match made {
                Ok(next) => {
                    open.with_mut(|held| {
                        if let Some(held) = held {
                            held.set = next;
                        }
                    });
                    failed.set(None);
                }
                Err(err) => failed.set(Some(format!("{err:#}"))),
            }
        });
    };

    rsx! {
        section { class: "panel",
            h3 {
                {icons::spark()}
                "Plan"
                span { class: "spacer" }
                span { class: "muted small hint",
                    match &first_name {
                        Some(name) => format!("from {name}, picked on the left"),
                        None => "pick a track on the left to start from, or describe it".to_string(),
                    }
                }
            }
            div { class: "form",
                div { class: "field wide",
                    label { "Sounding like" }
                    input {
                        placeholder: "e.g. dark rolling techno",
                        value: "{planned.like.clone().unwrap_or_default()}",
                        oninput: move |event| {
                            let text = event.value();
                            let mut open = open;
                            open.with_mut(|held| {
                                if let Some(held) = held {
                                    held.set.planned.like = Some(text).filter(|t| !t.trim().is_empty());
                                }
                            });
                        },
                    }
                }
                {number("Minutes", planned.minutes, "1", |p, v| p.minutes = v.max(1.0))}
                {number("± tempo %", planned.max_stretch * 100.0, "0.5", |p, v| p.max_stretch = (v / 100.0).clamp(0.005, 0.2))}
                {number("Beam", planned.beam as f64, "1", |p, v| p.beam = (v as usize).clamp(1, 64))}
                {number("Weakest mix", planned.min_transition, "0.05", |p, v| p.min_transition = v.clamp(0.0, 1.0))}
                button {
                    class: "primary",
                    disabled: busy().is_some() || (first.is_none() && planned.like.is_none()),
                    onclick: {
                        let library = library.clone();
                        move |_| plan(library.clone())
                    },
                    if empty { "Plan" } else { "Plan again" }
                }
            }
            if !empty {
                p { class: "muted small", "Planning again replaces the tracklist below." }
            }
        }
    }
}

/// Handing it over.
#[component]
fn Exporter(set: Set, library: Lib, unsaved: bool) -> Element {
    let state = use_context::<State>();
    let mut target = use_signal(|| Target::Rekordbox);
    let mut into =
        use_signal(|| std::env::var("HOME").map(|home| format!("{home}/Music/stelly-mix")).unwrap_or_else(|_| ".".into()));
    let exporting = use_signal(|| false);
    let done: Signal<Option<String>> = use_signal(|| None);

    let export = move |set: Set, library: Lib| {
        let (mut exporting, mut done) = (exporting, done);
        if exporting() {
            return;
        }
        exporting.set(true);
        done.set(None);
        let (folder, chosen) = (into(), target());
        spawn(async move {
            let options = export::Options { target: chosen, tracks: true, reference: true };
            let written = work::blocking(move || {
                work::export_set(&set, &library.0, std::path::Path::new(&folder), &options, &|line| eprintln!("{line}"))
            })
            .await;
            exporting.set(false);
            match written {
                Ok(w) => done.set(Some(format!("{} · {} files, {:.1} GB", w.dir.display(), w.files.len(), w.bytes as f64 / 1e9))),
                Err(err) => state.fail(err),
            }
        });
    };

    rsx! {
        section { class: "panel",
            h3 { {icons::export()} "Hand it over" }
            div { class: "form",
                div { class: "field",
                    label { "To" }
                    select {
                        value: "{target().as_str()}",
                        onchange: move |event| if let Ok(c) = event.value().parse::<Target>() { target.set(c) },
                        for option in Target::ALL {
                            option { value: "{option.as_str()}", "{option.title()}" }
                        }
                    }
                }
                div { class: "field wide",
                    label { "Into" }
                    input { value: "{into}", oninput: move |event| into.set(event.value()) }
                }
                button {
                    class: "secondary",
                    disabled: exporting() || set.tracks.is_empty(),
                    onclick: {
                        let (set, library) = (set.clone(), library.clone());
                        move |_| export(set.clone(), library.clone())
                    },
                    if exporting() { "exporting…" } else { "Export" }
                }
            }
            if let Some(caveat) = target().caveat() {
                p { class: "muted small", "{caveat}" }
            }
            if unsaved && !set.tracks.is_empty() {
                p { class: "muted small", "Exports what is on screen, kept or not." }
            }
            if let Some(where_to) = done() {
                p { class: "told", "Written to {where_to}" }
            }
        }
    }
}

/// How long a track is actually heard for: from where it came in to where
/// it goes out, which is not its length.
fn played(track: &stelly_mix::library::Track, step: Option<&stelly_mix::transitions::Transition>) -> f64 {
    match step {
        Some(step) => track.segments.get(step.out).map_or(track.analysis.seconds, |s| s.start),
        None => track.analysis.seconds,
    }
}
