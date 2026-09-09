use lagui::{DrawList, Quad, Rect};
use std::{hint::black_box, time::Instant};

fn main() {
    let mut storage = [Quad::default(); 4096];
    let mut list = DrawList::new(&mut storage);
    let mut samples = Vec::with_capacity(20_000);
    for sample in 0..22_000 {
        let start = Instant::now();
        list.clear();
        for i in 0..4096 {
            list.rectangle(
                Rect::new((i % 64) as f32 * 10., (i / 64) as f32 * 10., 9., 9.),
                [0.25, 0.5, 0.75, 1.],
            )
            .unwrap();
        }
        black_box(list.commands());
        let elapsed = start.elapsed().as_secs_f64() * 1e6;
        if sample >= 2000 {
            samples.push(elapsed);
        }
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "CPU construction only: 4096 quads; {} samples",
        samples.len()
    );
    println!(
        "p50 {:.3} us | p95 {:.3} us | p99 {:.3} us | max {:.3} us",
        samples[9999], samples[18999], samples[19799], samples[19999]
    );
    println!(
        "{} bytes command storage; no GPU, input, or text measured",
        size_of_val(&storage)
    );
}
