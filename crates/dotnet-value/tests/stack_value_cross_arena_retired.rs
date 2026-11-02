//! Structural evidence that the retired `StackValue::CrossArenaObjectRef(ObjectPtr, ArenaId)`
//! variant stays retired (commit 8e3768fe, checklist 4.5/4.6).
//!
//! The historical variant carried no registration generation, exposed provenance in
//! `Collect::trace`, and was dereferenced by the resolver without an identity-qualified lease.
//! Nothing in production constructed it, so there is no live producer whose ABA behavior could
//! be recreated; this scan stops a replacement from reappearing, in any workspace crate
//! (sources, tests, fuzz targets and benches). It is deliberately path-specific: it matches the
//! variant name only, never unrelated equality or exposure code, and `PointerOrigin::
//! CrossArenaObjectRef` (the identity-carrying ordinary origin) is allowed. Live-path ABA
//! rejection is covered by the origin/serialized-ABA tests in `pointer/tests.rs`.
//! It is source evidence, not VM execution, and runs natively in both feature configurations.
//! It is excluded from Miri (see the crate-level `cfg` below).

// Pure source-policy scan (no unsafe, no VM execution): it runs natively in both feature
// configurations, gives no Miri signal, and is far too slow when the workspace file walk is
// interpreted. cfg-excluded rather than `#[ignore]`d so no ignored test is reported.
#![cfg(not(miri))]

use std::{
    fs,
    path::{Path, PathBuf},
};

const VARIANT: &str = "CrossArenaObjectRef";

fn violations(label: &str, source: &str, is_stack_value_file: bool) -> Vec<String> {
    let mut out = Vec::new();
    for (idx, line) in source.lines().enumerate() {
        let code = line.trim_start();
        if code.starts_with("//") {
            continue;
        }
        let mut rest = code;
        let mut offset = 0;
        while let Some(pos) = rest.find(VARIANT) {
            let abs = offset + pos;
            let prefix = &code[..abs];
            let qualified_stack_value = prefix.ends_with("StackValue::");
            // Inside stack_value.rs the variant could also be declared bare or named `Self::`.
            let is_origin = prefix.ends_with("PointerOrigin::");
            if qualified_stack_value || (is_stack_value_file && !is_origin) {
                out.push(format!("{label}:{}: {code}", idx + 1));
                break;
            }
            offset = abs + VARIANT.len();
            rest = &code[offset..];
        }
    }
    out
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if name != "target" && name != ".git" {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn scanner_distinguishes_origin_from_retired_stack_variant() {
    assert!(violations("t", "PointerOrigin::CrossArenaObjectRef(_) => 1,", true).is_empty());
    assert!(violations("t", "// StackValue::CrossArenaObjectRef(p, _)", false).is_empty());
    assert!(violations("t", "x == y // unrelated expose_provenance equality", true).is_empty());
    assert_eq!(
        violations("t", "StackValue::CrossArenaObjectRef(p, t) => 1,", false).len(),
        1
    );
    assert_eq!(
        violations("t", "    CrossArenaObjectRef(ObjectPtr, ArenaId),", true).len(),
        1
    );
    assert_eq!(
        violations("t", "Self::CrossArenaObjectRef(p, _) => p,", true).len(),
        1
    );
    assert_eq!(
        violations(
            "t",
            "PointerOrigin::CrossArenaObjectRef(a) | Self::CrossArenaObjectRef(b)",
            true
        )
        .len(),
        1
    );
}

#[test]
fn stack_value_cross_arena_object_ref_stays_retired() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let this_file = Path::new(file!()).file_name().unwrap().to_owned();
    let mut files = Vec::new();
    rust_files(&crates, &mut files);
    assert!(
        files.len() > 100,
        "workspace scan found too few files: {}",
        files.len()
    );
    let mut found = Vec::new();
    let mut saw_stack_value = false;
    for path in files {
        if path.file_name() == Some(&this_file) {
            continue;
        }
        let is_sv = path.ends_with("dotnet-value/src/stack_value.rs");
        saw_stack_value |= is_sv;
        let source = fs::read_to_string(&path).unwrap();
        found.extend(violations(&path.display().to_string(), &source, is_sv));
    }
    assert!(saw_stack_value, "stack_value.rs was not scanned");
    assert!(
        found.is_empty(),
        "retired StackValue::CrossArenaObjectRef reappeared: {found:#?}"
    );
}
