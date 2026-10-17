//! How long the library takes to load with its scale, and how 30-minute sets
//! come out from a few tracks: how far they reach, and their transitions.
use stelly_mix_app::work;

fn main() -> anyhow::Result<()> {
    let t = std::time::Instant::now();
    let loaded = work::load()?;
    println!("load with scale: {:.2}s", t.elapsed().as_secs_f64());
    let seeds: Vec<usize> = (0..loaded.tracks.len())
        .filter(|&i| loaded.tracks[i].analysis.grid.as_ref().is_some_and(|g| g.steady) && loaded.tracks[i].segments.len() > 6)
        .step_by(97)
        .take(8)
        .collect();
    for floor in [0.5, 0.45, 0.4, 0.35] {
        println!("\nno transition under {floor}:");
        for &seed in &seeds {
            let options = stelly_mix::set::Options { min_transition: floor, ..Default::default() };
            let plan = work::plan(&loaded, seed, &options)?;
            let low = plan.transitions.iter().map(|t| t.score).fold(1.0, f64::min);
            println!("  {:<40.40}  {:>2} tracks, reaches {:>7}, mean {:.2}, lowest {:.2}", loaded.tracks[seed].name(), plan.tracks.len(), work::clock(plan.seconds), plan.mean_score(), low);
        }
    }
    Ok(())
}
