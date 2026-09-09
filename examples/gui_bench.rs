use best_gui::{
    Rect,
    layout::virtual_range,
    ui::{Role, Semantics, UiRuntimeStorage, WidgetId},
};
use std::{hint::black_box, time::Instant};

fn percentile(samples: &[f64], fraction: f64) -> f64 {
    samples[((samples.len() - 1) as f64 * fraction) as usize]
}

fn main() {
    const VISIBLE: usize = 512;
    let mut ui = UiRuntimeStorage::<VISIBLE>::default();
    let mut samples = Vec::with_capacity(10_000);
    for iteration in 0..11_000 {
        let start = Instant::now();
        ui.begin_frame();
        let visible = virtual_range(100_000, 24., 42_000., 768., 8);
        for (slot, row) in visible.enumerate() {
            let bounds = Rect::new(0., slot as f32 * 24., 800., 24.);
            ui.register_semantic(
                WidgetId(row as u64 + 1),
                bounds,
                Rect::new(0., 0., 800., 768.),
                true,
                Semantics {
                    role: Role::ListItem,
                    label: "virtual row",
                    ..Semantics::default()
                },
            )
            .unwrap();
        }
        black_box(ui.pointer_move([400., 380.]));
        ui.end_frame();
        if iteration >= 1_000 {
            samples.push(start.elapsed().as_secs_f64() * 1e6);
        }
    }
    samples.sort_by(f64::total_cmp);
    println!("virtual list frame: 100000 total rows, 512-region capacity");
    println!(
        "p50 {:.3} us | p95 {:.3} us | p99 {:.3} us | max {:.3} us",
        percentile(&samples, 0.50),
        percentile(&samples, 0.95),
        percentile(&samples, 0.99),
        samples[samples.len() - 1]
    );
}
