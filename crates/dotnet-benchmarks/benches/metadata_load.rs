//! Metadata-loading benchmark.
//!
//! Unlike `end_to_end`, which reuses a single `AssemblyLoader` across iterations (so the
//! expensive `System.Private.CoreLib` parse lands once in warmup and is never re-measured),
//! this benchmark parses assembly bytes fresh on every iteration. It isolates the cost of
//! `dotnetdll`'s metadata read pipeline, which is where the parallel (rayon) read work lives.
//!
//! Each case parses an already-in-memory, 8-byte-aligned byte buffer, so file I/O is excluded
//! and only the decode pipeline is timed. Two `ReadOptions` configurations are compared:
//!
//! - `lazy`: matches production (`lazy_method_bodies + lazy_method_signatures + lazy_attributes`).
//!   In this mode dotnetdll skips the eager parallel method-body decoder.
//! - `eager`: `ReadOptions::default()` — decodes bodies/signatures up front, using dotnetdll's
//!   rayon-parallel body decoder.
//!
//! Run with `RAYON_NUM_THREADS=1` vs unset to measure the parallel-read speedup:
//!
//! ```bash
//! cargo bench -p dotnet-benchmarks --bench metadata_load
//! RAYON_NUM_THREADS=1 cargo bench -p dotnet-benchmarks --bench metadata_load
//! ```

use std::{fs, hint::black_box, path::PathBuf};

use criterion::{Criterion, criterion_group, criterion_main};
use dotnetdll::prelude::{ReadOptions, Resolution};

/// Owns an 8-byte-aligned copy of an assembly's bytes (dotnetdll requires 8-byte alignment for
/// its pointer-cast reads — see `load_resolution_core` in `dotnet-assemblies`).
struct AlignedAssembly {
    name: String,
    _backing: Vec<u64>,
    len: usize,
}

impl AlignedAssembly {
    fn read(name: &str, path: &PathBuf) -> Self {
        let buf = fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        let len = buf.len();
        let cap = len.div_ceil(8);
        let mut backing = vec![0u64; cap];
        // SAFETY: `backing` has `cap * 8 >= len` bytes; src/dst are valid and non-overlapping.
        unsafe {
            std::ptr::copy_nonoverlapping(buf.as_ptr(), backing.as_mut_ptr() as *mut u8, len);
        }
        Self {
            name: name.to_string(),
            _backing: backing,
            len,
        }
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: `_backing` is valid for at least `len` bytes for the lifetime of `self`.
        unsafe { std::slice::from_raw_parts(self._backing.as_ptr() as *const u8, self.len) }
    }
}

fn lazy_opts() -> ReadOptions {
    ReadOptions {
        lazy_method_bodies: true,
        lazy_method_signatures: true,
        lazy_attributes: true,
        ..Default::default()
    }
}

fn eager_opts() -> ReadOptions {
    ReadOptions::default()
}

fn app_dir() -> PathBuf {
    dotnet_assemblies::find_dotnet_app_path().expect("could not find .NET shared framework path")
}

/// Assemblies to load in the "framework set" case — a realistic cold-start working set.
const FRAMEWORK_SET: &[&str] = &[
    "System.Private.CoreLib",
    "System.Runtime",
    "System.Console",
    "System.Collections",
    "System.Linq",
    "System.Text.Json",
];

fn bench_corlib(c: &mut Criterion) {
    let dir = app_dir();
    let corlib = AlignedAssembly::read(
        "System.Private.CoreLib",
        &dir.join("System.Private.CoreLib.dll"),
    );

    let mut group = c.benchmark_group("load_corlib");
    // corlib is large and slow to parse; keep sample counts honest but bounded.
    group.sample_size(20);

    group.bench_function("lazy", |b| {
        let opts = lazy_opts();
        b.iter(|| {
            let res = Resolution::parse(black_box(corlib.bytes()), opts)
                .expect("failed to parse corlib (lazy)");
            black_box(&res);
        });
    });

    group.bench_function("eager", |b| {
        let opts = eager_opts();
        b.iter(|| {
            let res = Resolution::parse(black_box(corlib.bytes()), opts)
                .expect("failed to parse corlib (eager)");
            black_box(&res);
        });
    });

    group.finish();
}

fn bench_framework_set(c: &mut Criterion) {
    let dir = app_dir();
    let assemblies: Vec<AlignedAssembly> = FRAMEWORK_SET
        .iter()
        .map(|name| AlignedAssembly::read(name, &dir.join(format!("{name}.dll"))))
        .collect();

    let mut group = c.benchmark_group("load_framework_set");
    group.sample_size(20);

    group.bench_function("lazy", |b| {
        let opts = lazy_opts();
        b.iter(|| {
            for asm in &assemblies {
                let res = Resolution::parse(black_box(asm.bytes()), opts)
                    .unwrap_or_else(|e| panic!("failed to parse {} (lazy): {e}", asm.name));
                black_box(&res);
            }
        });
    });

    group.bench_function("eager", |b| {
        let opts = eager_opts();
        b.iter(|| {
            for asm in &assemblies {
                let res = Resolution::parse(black_box(asm.bytes()), opts)
                    .unwrap_or_else(|e| panic!("failed to parse {} (eager): {e}", asm.name));
                black_box(&res);
            }
        });
    });

    group.finish();
}

criterion_group! {
    name = metadata_load;
    config = Criterion::default().configure_from_args();
    targets = bench_corlib, bench_framework_set
}
criterion_main!(metadata_load);
