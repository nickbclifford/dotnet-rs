//! # dotnet-rs
//!
//! An experimental .NET runtime interpreter written in Rust.
//!
//! ## Feature Flags
//!
//! - `multithreading`: Enables OS-thread execution support (including .NET `Thread`,
//!   `Monitor`, and stop-the-world GC coordination across per-thread arenas). Pulls in
//!   `parking_lot`.
#![allow(clippy::arc_with_non_send_sync)]
use clap::Parser;
use dotnet_types::{TypeDescription, members::MethodDescription};
use dotnet_vm::{self as vm, ExecutorResult};
use dotnetdll::prelude::*;
use std::process::ExitCode;
use vm::{state, sync::Arc};

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "An experimental interpreter for the .NET runtime"
)]
pub struct Args {
    #[arg(short, long, value_name = "FOLDER")]
    pub assemblies: String,
    #[arg(value_name = "DLL")]
    pub entrypoint: String,
}

/// Initialize the rayon global thread pool to a bounded size before any assembly parsing begins.
///
/// dotnetdll fans each per-assembly metadata table across all CPU cores via rayon. On many-core
/// hosts the fork/join + work-stealing coordination dominates actual decode work (measured: >50%
/// of one-shot CPU in rayon/crossbeam machinery). A small pool is ~12% faster end-to-end than
/// all-cores and ~30% faster than single-threaded. The VM and lazy on-demand decode are
/// single-threaded and unaffected. Respects RAYON_NUM_THREADS if set.
fn init_rayon_pool() {
    let n = std::env::var("RAYON_NUM_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get().min(4))
                .unwrap_or(1)
        });
    // build_global() errors if already initialized; first-call-wins is fine.
    let _ = rayon::ThreadPoolBuilder::new().num_threads(n).build_global();
}

pub fn run_cli() -> ExitCode {
    init_rayon_pool();
    let args = Args::parse();

    let loader = match dotnet_assemblies::AssemblyLoader::new(args.assemblies) {
        Ok(l) => Arc::new(l),
        Err(e) => {
            eprintln!("Error initializing assembly loader: {}", e);
            return ExitCode::from(1);
        }
    };

    let resolution = match loader.load_resolution_from_file(args.entrypoint) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error loading entry point: {}", e);
            return ExitCode::from(1);
        }
    };

    let entry_method = match resolution.entry_point {
        Some(EntryPoint::Method(m)) => m,
        Some(EntryPoint::File(f)) => {
            eprintln!(
                "Entry-point files are not supported: {}",
                resolution[f].name
            );
            return ExitCode::from(1);
        }
        None => {
            eprintln!("Expected input module to have an entry point, received one without");
            return ExitCode::from(1);
        }
    };

    #[allow(clippy::arc_with_non_send_sync)]
    let shared = Arc::new(state::SharedGlobalState::new(loader));
    let mut executor = vm::Executor::new(shared);

    let td = TypeDescription::new(resolution.clone(), entry_method.parent_type());
    let Some(method_index) = td
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, &resolution.definition()[entry_method]))
    else {
        eprintln!("Internal VM error: failed to locate entry method index");
        return ExitCode::from(1);
    };

    let entrypoint = MethodDescription::new(
        td,
        vm::GenericLookup::default(),
        resolution,
        MethodMemberIndex::Method(method_index),
    );
    if let Err(e) = executor.entrypoint(entrypoint) {
        eprintln!("Internal VM error: {}", e);
        return ExitCode::from(1);
    }

    let result = executor.run();
    match result {
        ExecutorResult::Exited(i) => exit_fast(i as i32),
        ExecutorResult::Threw(exc) => {
            eprintln!("{}", exc);
            exit_fast(1);
        }
        ExecutorResult::Error(e) => {
            eprintln!("Internal VM error: {}", e);
            exit_fast(1);
        }
    }
}

/// Flush stdio and exit without running destructors.
///
/// Skipping Drop on arena / cache structures is pure overhead for a CLI process that is
/// about to exit anyway.  The in-process integration-test harness never calls `run_cli`,
/// so leaking is contained to real one-shot CLI subprocesses.
fn exit_fast(code: i32) -> ! {
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(code)
}
