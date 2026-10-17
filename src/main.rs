//! stelly-mix with local files only. A build with more sources is this with
//! them passed to `run`; see `stelly_mix::source`.

fn main() -> anyhow::Result<()> {
    stelly_mix::cli::run(Vec::new())
}
