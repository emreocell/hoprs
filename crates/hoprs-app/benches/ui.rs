//! Spec §9, "100 000 entries scroll at ≥ 60 fps": one frame of a tab whose pane lists
//! 100 000 entries, at a random scroll offset: `view()`, the layout of the virtual list
//! (which builds the rows in view) and the draw (tiny-skia, recorded, not rasterized).
//!
//! Criterion reports the mean; then a fixed run of [`P95_FRAMES`] frames gives the p95,
//! printed as `ui_frame_p95_ms=<x>`. The p95 must stay under one 60 fps frame (16 ms),
//! or the bench exits with status 1.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::Criterion;
use hoprs_app::bench::FileList;

const ENTRIES: usize = 100_000;
const P95_FRAMES: usize = 1_000;
const FRAME_BUDGET: Duration = Duration::from_millis(16);

/// Offsets spread over the whole list, the same on every run (xorshift64).
struct Offsets {
    state: u64,
    max: f32,
}

impl Offsets {
    fn new(max: f32) -> Self {
        Self {
            state: 0x9E37_79B9_7F4A_7C15,
            max,
        }
    }

    fn next(&mut self) -> f32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        (self.state >> 40) as f32 / (1u64 << 24) as f32 * self.max
    }
}

fn main() {
    let mut list = FileList::new(ENTRIES);
    let mut offsets = Offsets::new(list.max_offset());
    // Warm the widget tree and the text caches, as a window that is already open.
    list.frame(0.0);

    let mut c = Criterion::default().configure_from_args();
    c.bench_function("files_100k_frame_random_offset", |b| {
        b.iter(|| list.frame(black_box(offsets.next())));
    });
    c.final_summary();

    let mut frames: Vec<Duration> = (0..P95_FRAMES)
        .map(|_| {
            let offset = offsets.next();
            let start = Instant::now();
            list.frame(offset);
            start.elapsed()
        })
        .collect();
    frames.sort();
    let p95 = frames[P95_FRAMES * 95 / 100];
    let max = frames[P95_FRAMES - 1];
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let ok = p95 < FRAME_BUDGET;
    println!(
        "ui_frame_p95_ms={:.3} ui_frame_max_ms={:.3} entries={ENTRIES}",
        ms(p95),
        ms(max)
    );
    println!("\n### File list ({P95_FRAMES} frames at random offsets, 100 000 entries)\n");
    println!("| Target (spec §9) | Measured | Criterion | Result |");
    println!("|---|---|---|---|");
    println!(
        "| 100k entries scroll at ≥ 60 fps | p95 {:.2} ms per frame (max {:.2} ms) | p95 < 16 ms | {} |",
        ms(p95),
        ms(max),
        if ok { "pass" } else { "**FAIL**" }
    );
    if !ok {
        std::process::exit(1);
    }
}
