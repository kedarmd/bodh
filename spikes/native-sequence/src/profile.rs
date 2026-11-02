//! Native CPU redraw benchmark. This is diagnostic tooling, not a product limit.
use super::*;
use std::time::Instant;

pub fn run(handle: WindowHandle<Demo>, cx: &mut App) {
    let handle: AnyWindowHandle = handle.into();
    cx.defer(move |cx| {
        handle
            .update(cx, |_, window, _| {
                println!("native CPU viewport: {:?}", window.viewport_size())
            })
            .unwrap();
        let mut slow = false;
        for active in 0..3 {
            for resize in [false, true] {
                let mut samples = Vec::new();
                for frame in 0..12 {
                    let start = Instant::now();
                    let clear = handle
                        .update(cx, |root, window, cx| {
                            let entity = root.downcast::<Demo>().unwrap();
                            entity.update(cx, |view, cx| {
                                view.active = active;
                                if resize {
                                    view.inspector_width = 340. + (frame % 4) as f32 * 40.;
                                } else {
                                    view.select(
                                        frame % view.flows[active].flow.interactions().len(),
                                        cx,
                                    );
                                }
                                cx.notify();
                            });
                            window.refresh();
                            window.draw(cx)
                        })
                        .unwrap();
                    clear.clear();
                    if frame >= 2 {
                        samples.push(start.elapsed().as_secs_f64() * 1000.);
                    }
                }
                samples.sort_by(f64::total_cmp);
                let p50 = samples[samples.len() / 2];
                let p95 = *samples.last().unwrap();
                println!(
                    "flow={active} operation={} native_cpu_ms p50={p50:.3} p95={p95:.3}",
                    if resize { "resize" } else { "select" }
                );
                slow |= p95 > 16.7;
            }
        }
        println!(
            "16.7ms diagnostic frame budget: {} (excludes compositor/GPU presentation)",
            if slow { "EXCEEDED" } else { "MET" }
        );
        cx.quit();
    });
}
