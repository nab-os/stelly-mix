//! How long the library takes to read, to scale, and to answer `next`.
fn main() -> anyhow::Result<()> {
    let t = std::time::Instant::now();
    let tracks = stelly_mix::library::Library::open(stelly_mix::library::Library::default_dir())?.all()?;
    let read = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    let scale = stelly_mix::transitions::SoundScale::of(&tracks);
    let scaled = t.elapsed().as_secs_f64();
    let first = tracks.iter().position(|t| t.analysis.grid.as_ref().is_some_and(|g| g.steady)).unwrap();
    let t = std::time::Instant::now();
    let options = stelly_mix::transitions::Options { sound: scale, ..Default::default() };
    let ways = stelly_mix::transitions::from(&tracks[first], &tracks, &options);
    println!("{} tracks: read {read:.2}s, scale {scaled:.2}s, next {:.2}s ({} ways)", tracks.len(), t.elapsed().as_secs_f64(), ways.len());
    Ok(())
}
