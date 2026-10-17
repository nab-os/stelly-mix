//! stelly-mix in a window, over local files, and Stelly's server's mix
//! library when `STELLY_SERVER` and `STELLY_TOKEN` say where it is.

// Without this a release build on Windows opens a console next to the window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    stelly_mix_app::run(stelly_mix::stelly::sources());
}
