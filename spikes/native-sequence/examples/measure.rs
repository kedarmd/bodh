use bodh_sequence_spike::{Scene, stress};
use std::time::Instant;
fn main() {
    for (participants, steps) in [(3, 7), (12, 120), (24, 1000), (48, 10000)] {
        let flow = stress(participants, steps);
        let start = Instant::now();
        let scene = Scene::layout(&flow);
        let elapsed = start.elapsed();
        let hit_start = Instant::now();
        let mut hits = 0;
        for row in &scene.rows {
            hits += usize::from(scene.hit(row.rect.x + 1., row.rect.y + 1.).is_some());
        }
        println!(
            "{participants} participants / {steps} steps: {} x {} px, layout {elapsed:?}, {hits} hits {:?}",
            scene.width,
            scene.height,
            hit_start.elapsed()
        );
    }
}
