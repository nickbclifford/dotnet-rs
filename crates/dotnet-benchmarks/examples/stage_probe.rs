//! Per-stage parse timing probe.
//!
//! Captures dotnetdll's `stage-timing` `tracing::debug!` events and prints a per-stage breakdown
//! for a lazy vs eager parse of `System.Private.CoreLib`. Build with `--features stage-timing`:
//!
//! ```bash
//! cargo run --release --example stage_probe -p dotnet-benchmarks --features stage-timing
//! RAYON_NUM_THREADS=1 cargo run --release --example stage_probe -p dotnet-benchmarks --features stage-timing
//! ```
//!
//! Without `stage-timing` the parser emits no stage events and this prints empty tables.

use std::{
    collections::BTreeMap,
    fs,
    sync::{Arc, Mutex},
};

use dotnetdll::prelude::{ReadOptions, Resolution};
use tracing::field::{Field, Visit};
use tracing_subscriber::{Layer, layer::Context, prelude::*, registry::Registry};

#[derive(Default)]
struct StageTotals {
    // stage name -> (total_ns, count)
    map: BTreeMap<String, (u64, u64)>,
}

struct StageLayer {
    totals: Arc<Mutex<StageTotals>>,
}

struct StageVisitor {
    stage: Option<String>,
    elapsed_ns: Option<u64>,
}

impl Visit for StageVisitor {
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "elapsed_ns" {
            self.elapsed_ns = Some(value);
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "stage" {
            self.stage = Some(value.to_string());
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "stage" && self.stage.is_none() {
            self.stage = Some(format!("{value:?}").trim_matches('"').to_string());
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for StageLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut v = StageVisitor {
            stage: None,
            elapsed_ns: None,
        };
        event.record(&mut v);
        if let (Some(stage), Some(ns)) = (v.stage, v.elapsed_ns) {
            let mut t = self.totals.lock().unwrap();
            let e = t.map.entry(stage).or_insert((0, 0));
            e.0 += ns;
            e.1 += 1;
        }
    }
}

fn aligned(path: &str) -> (Vec<u64>, usize) {
    let buf = fs::read(path).unwrap();
    let len = buf.len();
    let mut backing = vec![0u64; len.div_ceil(8)];
    unsafe {
        std::ptr::copy_nonoverlapping(buf.as_ptr(), backing.as_mut_ptr() as *mut u8, len);
    }
    (backing, len)
}

fn run(label: &str, bytes: &[u8], opts: ReadOptions, iters: u32) {
    let totals = Arc::new(Mutex::new(StageTotals::default()));
    let layer = StageLayer {
        totals: totals.clone(),
    };
    let subscriber = Registry::default().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        for _ in 0..iters {
            let res = Resolution::parse(bytes, opts).expect("parse failed");
            std::hint::black_box(&res);
        }
    });

    let t = totals.lock().unwrap();
    let threads = std::env::var("RAYON_NUM_THREADS").unwrap_or_else(|_| "default".into());
    println!("\n=== {label}  (RAYON_NUM_THREADS={threads}, iters={iters}) ===");
    if t.map.is_empty() {
        println!("  (no stage events — build with --features stage-timing)");
        return;
    }
    let mut grand = 0u64;
    for (stage, (ns, count)) in &t.map {
        let avg_ms = (*ns as f64 / *count as f64) / 1e6;
        grand += ns / count.max(&1);
        println!("  {stage:<24} {avg_ms:>8.3} ms/iter");
    }
    println!(
        "  {:<24} {:>8.3} ms/iter (sum of stages)",
        "TOTAL",
        grand as f64 / 1e6
    );
}

fn main() {
    let dir = dotnet_assemblies::find_dotnet_app_path().expect("no .NET shared path");
    let corlib = dir.join("System.Private.CoreLib.dll");
    let (backing, len) = aligned(corlib.to_str().unwrap());
    let bytes = unsafe { std::slice::from_raw_parts(backing.as_ptr() as *const u8, len) };

    let lazy = ReadOptions {
        lazy_method_bodies: true,
        lazy_method_signatures: true,
        lazy_attributes: true,
        ..Default::default()
    };
    run("corlib LAZY", bytes, lazy, 10);
    run("corlib EAGER", bytes, ReadOptions::default(), 5);
}
