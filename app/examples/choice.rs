//! How the next tracks are chosen, measured: for some steady tracks, what the
//! best candidates' sound is, and how much of their score sound decides.
use stelly_mix_app::work;

fn main() -> anyhow::Result<()> {
    let loaded = work::load()?;
    println!("scale {:?}", loaded.scale);
    let seeds: Vec<usize> = (0..loaded.tracks.len())
        .filter(|&i| loaded.tracks[i].analysis.grid.as_ref().is_some_and(|g| g.steady) && loaded.tracks[i].segments.len() > 6)
        .step_by(97)
        .take(8)
        .collect();
    for seed in seeds {
        let ways = work::next(&loaded, seed, 0.06);
        let top: Vec<_> = ways.iter().take(10).collect();
        let by_sound = {
            let mut s: Vec<f64> = ways.iter().map(|w| w.sound).collect();
            s.sort_by(|a, b| b.total_cmp(a));
            s
        };
        let rank_of = |w: &stelly_mix::transitions::Transition| by_sound.iter().position(|&s| s == w.sound).unwrap() + 1;
        println!("\n{} ({} candidates; sound: best {:.2}, 10th {:.2}, median {:.2})",
            loaded.tracks[seed].name(), ways.len(), by_sound[0], by_sound.get(9).copied().unwrap_or(0.0), by_sound.get(by_sound.len() / 2).copied().unwrap_or(0.0));
        for w in top.iter().take(5) {
            println!("  {:.2}  sound {:.2} (#{} by sound)  key {:?}  {}", w.score, w.sound, rank_of(w), w.key_steps, loaded.tracks[w.to].name());
        }
    }
    Ok(())
}
