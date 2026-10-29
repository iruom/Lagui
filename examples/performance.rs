//! CPU/allocator benchmark, deliberately separate from GPU/presentation latency.
use lagui::{
    Rect,
    ui::{UiRuntimeStorage, WidgetId},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};

struct CountingAllocator;
static TRACK: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicU64 = AtomicU64::new(0);
// SAFETY: the allocator delegates all pointer/size/alignment handling to System.
// Atomic counters neither allocate nor inspect allocation contents.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            CALLS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            CALLS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn measure(name: &str, count: usize, mut frame: impl FnMut()) {
    for _ in 0..100 {
        frame();
    }
    let mut samples = Vec::with_capacity(1000);
    CALLS.store(0, Ordering::Relaxed);
    for _ in 0..1000 {
        TRACK.store(true, Ordering::Relaxed);
        let start = Instant::now();
        frame();
        let elapsed = start.elapsed().as_secs_f64() * 1e6;
        TRACK.store(false, Ordering::Relaxed);
        samples.push(elapsed);
    }
    samples.sort_by(f64::total_cmp);
    let allocations = CALLS.load(Ordering::Relaxed);
    println!(
        "{{\"case\":\"{name}\",\"regions\":{count},\"samples\":1000,\"p50_us\":{:.3},\"p95_us\":{:.3},\"p99_us\":{:.3},\"max_us\":{:.3},\"warm_allocation_calls\":{allocations},\"os\":\"{}\",\"arch\":\"{}\"}}",
        samples[500],
        samples[950],
        samples[990],
        samples[999],
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    assert_eq!(
        allocations, 0,
        "warmed UI input/registration must reuse its storage"
    );
}
fn main() {
    let clip = Rect::new(0., 0., 100., 100.);
    for count in [32, 512, 10_000] {
        for sparse in [false, true] {
            let mut ui = UiRuntimeStorage::<0>::default();
            measure(
                if sparse {
                    "unique-sparse"
                } else {
                    "unique-sequential"
                },
                count,
                || {
                    ui.begin_frame();
                    for id in 0..count as u64 {
                        let id = if sparse {
                            id.wrapping_mul(0x9e3779b97f4a7c15)
                        } else {
                            id
                        };
                        ui.register_unique(WidgetId(id), clip, clip, true).unwrap();
                    }
                    black_box(ui.pointer_move([50., 50.]));
                    ui.end_frame();
                },
            );
        }
    }
    let mut ui = UiRuntimeStorage::<0>::default();
    ui.begin_frame();
    for id in 0..10_000 {
        ui.register_unique(WidgetId(id), clip, clip, true).unwrap();
    }
    ui.end_frame();
    measure("hover-only", 10_000, || {
        black_box(ui.pointer_move([50., 50.]));
    });
    // Charge registration and derived-index maintenance to each frame as well
    // as measuring isolated queries. All modes must remain allocation-free
    // once storage has warmed, including when every bound moves.
    for mode in ["grid-static", "grid-one-moved", "grid-all-moved"] {
        let mut ui = UiRuntimeStorage::<0>::default();
        let mut frame = 0;
        let clip = Rect::new(0., 0., 1200., 1200.);
        measure(mode, 10_000, || {
            ui.begin_frame();
            for i in 0..10_000 {
                let dx = if mode == "grid-all-moved" || mode == "grid-one-moved" && i == 0 {
                    (frame % 2) as f32
                } else {
                    0.
                };
                ui.register_unique(
                    WidgetId(i),
                    Rect::new((i % 100) as f32 * 10. + dx, (i / 100) as f32 * 10., 8., 8.),
                    clip,
                    true,
                )
                .unwrap();
            }
            ui.end_frame();
            black_box(ui.pointer_move([1001., 1001.]));
            frame += 1;
        });
    }
    let mut ui = UiRuntimeStorage::<0>::default();
    measure("duplicate-regions", 10_000, || {
        ui.begin_frame();
        for id in 0..10_000 {
            ui.register(WidgetId(id / 2), clip, clip, true).unwrap();
        }
        ui.end_frame();
    });
}
