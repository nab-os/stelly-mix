//! Where named tracks sit in the library on a description and on energy.
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let description = args.next().expect("a description");
    let names: Vec<String> = args.collect();
    let tracks = stelly_mix::library::Library::open(stelly_mix::library::Library::default_dir())?.all()?;
    let mut encoder = stelly_mix::text::TextEncoder::load(&stelly_mix::models::dir())?;
    let like = stelly_mix::text::scores(&mut encoder, &tracks, &description)?;
    let onset = |t: &stelly_mix::library::Track| {
        let v: Vec<f64> = t.segments.iter().filter_map(|s| s.onset_rate).collect();
        v.iter().sum::<f64>() / v.len().max(1) as f64
    };
    let mut onsets: Vec<f64> = tracks.iter().map(onset).collect();
    onsets.sort_by(f64::total_cmp);
    let pct = |v: f64| onsets.iter().filter(|&&o| o < v).count() as f64 / onsets.len() as f64;
    let near120 = tracks.iter().zip(&like).filter(|(t, _)| t.bpm().is_some_and(|b| (b / 120.0 - 1.0).abs() <= 0.06)).count();
    let chill_near = tracks.iter().zip(&like).filter(|(t, l)| **l >= 0.85 && t.bpm().is_some_and(|b| (b / 120.0 - 1.0).abs() <= 0.06)).count();
    println!("{} tracks; {} within 6% of 120 BPM; {} of those in the top 15% for the description", tracks.len(), near120, chill_near);
    for name in names {
        if let Some((t, l)) = tracks.iter().zip(&like).find(|(t, _)| t.name().contains(&name)) {
            println!("{:.2} likeness  {:.0}th pct busyness  {}", l, pct(onset(t)) * 100.0, t.name());
        }
    }
    Ok(())
}
