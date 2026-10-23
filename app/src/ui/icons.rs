//! Line icons, drawn inline rather than as characters: `▶` and `×` are a
//! font's idea of those shapes, and WebKitGTK here and a webview elsewhere
//! render them as two different pictures at two different weights.
//!
//! A 24px grid, stroked with `currentColor`, so a button's colour is its
//! icon's. Sizing is the caller's, through `.icon` in the stylesheet.

use dioxus::prelude::*;

fn stroked(body: Element) -> Element {
    rsx! {
        svg {
            class: "icon",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            {body}
        }
    }
}

fn filled(body: Element) -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "currentColor", {body} }
    }
}


/// Filled, because it is the one control that is pressed rather than read.
pub fn play() -> Element {
    filled(rsx! { path { d: "M8 5.2v13.6a1 1 0 0 0 1.52.85l11-6.8a1 1 0 0 0 0-1.7l-11-6.8A1 1 0 0 0 8 5.2z" } })
}

pub fn pause() -> Element {
    filled(rsx! {
        rect { x: "6", y: "5", width: "4", height: "14", rx: "1" }
        rect { x: "14", y: "5", width: "4", height: "14", rx: "1" }
    })
}

/// One track: its waveform. Four bars, not five — at tab size the fifth
/// closes the gaps and the whole thing greys over.
pub fn track() -> Element {
    stroked(rsx! {
        path { d: "M4 9.5v5M10 4v16M16 7v10M21 10.5v3" }
    })
}

/// A set: one track running into the next, drawn as two long bars that
/// overlap where the mix happens. Two shapes, because three of them at tab
/// size is a smudge.
pub fn set() -> Element {
    stroked(rsx! {
        rect { x: "2", y: "6", width: "14", height: "5", rx: "2.5" }
        rect { x: "8", y: "14", width: "14", height: "5", rx: "2.5" }
    })
}

/// The sets kept: a list of them. Deliberately not the `set` mark again at
/// another size — side by side in the tabs, two stacks of bars were one
/// picture twice.
pub fn sets() -> Element {
    stroked(rsx! {
        path { d: "M4 7h16M4 12h16M4 17h9" }
    })
}

/// Finding by description: a magnifier over words.
pub fn find() -> Element {
    stroked(rsx! {
        circle { cx: "11", cy: "11", r: "6" }
        path { d: "M20 20l-4.3-4.3" }
    })
}

/// Handing it over: out of the window and onto something else.
pub fn export() -> Element {
    stroked(rsx! {
        path { d: "M12 3v12" }
        path { d: "M8 7l4-4 4 4" }
        path { d: "M4 15v4a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-4" }
    })
}






/// A copy of a set, made to change without losing the first.
pub fn duplicate() -> Element {
    stroked(rsx! {
        rect { x: "9", y: "9", width: "12", height: "12", rx: "2" }
        path { d: "M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1" }
    })
}

/// Renaming: a pencil.
pub fn rename() -> Element {
    stroked(rsx! {
        path { d: "M4 20h4L19 9a2.1 2.1 0 0 0-3-3L5 17z" }
        path { d: "M15 6l3 3" }
    })
}




/// Moving a track up or down the set.
pub fn up() -> Element {
    stroked(rsx! { path { d: "M12 19V6M6 12l6-6 6 6" } })
}

pub fn down() -> Element {
    stroked(rsx! { path { d: "M12 5v13M6 12l6 6 6-6" } })
}

/// Taking a track out of the set.
pub fn remove() -> Element {
    stroked(rsx! { path { d: "M5 12h14" } })
}

/// Putting something else in this slot: one going out, another coming in.
pub fn swap() -> Element {
    stroked(rsx! {
        path { d: "M4 8h13M14 5l3 3-3 3" }
        path { d: "M20 16H7M10 13l-3 3 3 3" }
    })
}

/// The server the music comes from.
pub fn server() -> Element {
    stroked(rsx! {
        rect { x: "3", y: "4", width: "18", height: "7", rx: "2" }
        rect { x: "3", y: "13", width: "18", height: "7", rx: "2" }
        path { d: "M7 7.5h.01M7 16.5h.01" }
    })
}

/// Analysing: a track being read through.
pub fn analyse() -> Element {
    stroked(rsx! {
        path { d: "M3 12h4l3-7 4 14 3-7h4" }
    })
}

/// Settings.
pub fn settings() -> Element {
    stroked(rsx! {
        circle { cx: "12", cy: "12", r: "3.2" }
        path { d: "M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2.2 2.2M16.8 16.8L19 19M19 5l-2.2 2.2M7.2 16.8L5 19" }
    })
}

