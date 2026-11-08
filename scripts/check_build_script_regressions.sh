#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

LOG_DIR="${TMPDIR:-/tmp}/dotnet-rs-build-script-probes"
mkdir -p "$LOG_DIR"

fail() {
    echo "ERROR: $*" >&2
    exit 1
}

log_step() {
    echo
    echo "==> $*"
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

print_log_excerpt() {
    local log_file="$1"
    echo "---- tail: $log_file ----" >&2
    if [[ -f "$log_file" ]]; then
        tail -n 200 "$log_file" >&2
    else
        echo "(missing log file)" >&2
    fi
    echo "-------------------------" >&2
}

run_logged() {
    local log_file="$1"
    shift
    if ! "$@" >"$log_file" 2>&1; then
        print_log_excerpt "$log_file"
        fail "command failed: $* (see $log_file)"
    fi
}

assert_vm_rerun_marker_present() {
    local log_file="$1"
    local label="$2"
    if rg -n 'Dirty dotnet-vm .*src/intrinsics' "$log_file" >/dev/null; then
        return
    fi
    if rg -n 'build/dotnet-vm-.*/build-script-build' "$log_file" >/dev/null; then
        return
    fi
    print_log_excerpt "$log_file"
    fail "${label}-file rerun marker missing in $log_file"
}

support_dll_path() {
    find target -path '*/build/dotnet-assemblies-*/out/support.dll' -type f | head -n1
}

assert_generated_support_slot_artifacts() {
    local dll_path="$1"
    local out_dir
    out_dir="$(dirname "$dll_path")"
    local rust_slots="$out_dir/support_slots.rs"
    local csharp_slots="$out_dir/support-generated/RuntimeSlotId.g.cs"

    [[ -f "$rust_slots" ]] || fail "generated Rust slot descriptors missing at $rust_slots"
    [[ -f "$csharp_slots" ]] || fail "generated C# runtime-slot enum missing at $csharp_slots"
    rg -q 'pub enum RuntimeSlotId' "$rust_slots" \
        || fail "generated Rust slot IDs missing from $rust_slots"
    rg -q 'RuntimeTypeHandleValue = 1' "$rust_slots" \
        || fail "generated Rust slot IDs have unexpected contents in $rust_slots"
    rg -q 'internal enum RuntimeSlotId' "$csharp_slots" \
        || fail "generated C# runtime-slot enum missing from $csharp_slots"
    rg -q 'ReadOnlySpanLength = 21' "$csharp_slots" \
        || fail "generated C# slot IDs have unexpected contents in $csharp_slots"
}

assert_clippy_skips_dotnet() {
    local log_file="$LOG_DIR/clippy-clean.log"
    log_step "Probe 1/4: clean clippy should skip dotnet/MSBuild invocation paths"
    cargo clean
    cargo clippy --workspace --all-targets 2>&1 | tee "$log_file"

    if rg -n -i 'dotnet (build|restore)|msbuild|csproj' "$log_file" >/dev/null; then
        fail "detected dotnet/MSBuild invocation markers during clean clippy (see $log_file)"
    fi
}

assert_skip_toggle_invalidates_support_build() {
    local skip_log="$LOG_DIR/assemblies-skip.log"
    local noskip_log="$LOG_DIR/assemblies-noskip-after-skip.log"
    local dll_path=""
    local size=""

    log_step "Probe 2/4: DOTNET_SKIP_BUILD toggle must invalidate dotnet-assemblies output"
    cargo clean
    DOTNET_SKIP_BUILD=1 cargo build -p dotnet-assemblies 2>&1 | tee "$skip_log"

    dll_path="$(support_dll_path)"
    [[ -n "$dll_path" ]] || fail "support.dll was not produced under skip mode"
    size="$(stat -c%s "$dll_path")"
    [[ "$size" == "0" ]] || fail "expected stub support.dll size 0 in skip mode, got $size at $dll_path"
    assert_generated_support_slot_artifacts "$dll_path"

    cargo build -p dotnet-assemblies -vv 2>&1 | tee "$noskip_log"
    rg -n 'env variable DOTNET_SKIP_BUILD changed' "$noskip_log" >/dev/null \
        || fail "missing DOTNET_SKIP_BUILD env invalidation marker in -vv log ($noskip_log)"

    dll_path="$(support_dll_path)"
    [[ -n "$dll_path" ]] || fail "support.dll missing after non-skip rebuild"
    size="$(stat -c%s "$dll_path")"
    [[ "$size" -gt 0 ]] || fail "expected non-empty support.dll after unsetting skip, got $size at $dll_path"
    assert_generated_support_slot_artifacts "$dll_path"
}

assert_vm_directory_rerun_invalidation() {
    local baseline_log="$LOG_DIR/vm-rerun-baseline.log"
    local add_log="$LOG_DIR/vm-rerun-add.log"
    local remove_log="$LOG_DIR/vm-rerun-remove.log"
    # Keep the tracked __rerun_probe.rs fixture intact; this distinct temporary
    # filename exercises the same watched directory add/remove behavior.
    local probe_file="crates/dotnet-vm/src/intrinsics/.build_script_rerun_probe.rs"
    local probe_dir
    probe_dir="$(dirname "$probe_file")"

    log_step "Probe 3/4: dotnet-vm rerun-if-changed should retrigger on add/remove under watched root"
    cargo clean
    run_logged "$baseline_log" cargo build -p dotnet-vm --no-default-features -vv

    rm -f "$probe_file"
    cat >"$probe_file" <<'EOF'
// Temporary probe file for build-script invalidation test.
EOF
    sleep 1
    touch "$probe_dir"

    run_logged "$add_log" cargo build -p dotnet-vm --no-default-features -vv
    assert_vm_rerun_marker_present "$add_log" "add"

    rm -f "$probe_file"
    sleep 1
    touch "$probe_dir"

    run_logged "$remove_log" cargo build -p dotnet-vm --no-default-features -vv
    assert_vm_rerun_marker_present "$remove_log" "remove"
}

# Compile actual scanned handlers, including an external-crate selector with generic host bounds.
assert_async_intrinsic_wiring() (
    set -euo pipefail
    local root="crates/dotnet-intrinsics-string/src"
    local lib="$root/lib.rs" probe="$root/async_build_probe.rs"
    local conflict="crates/dotnet-vm/src/intrinsics/.async_selector_conflict.rs"
    local backup test_backup vm_backup test_file="crates/dotnet-vm/src/native_substring_tests.rs"
    local vm_lib="crates/dotnet-vm/src/intrinsics/mod.rs" vm_probe="crates/dotnet-vm/src/intrinsics/async_nested_probe.rs"
    backup="$(mktemp)"
    test_backup="$(mktemp)"
    vm_backup="$(mktemp)"
    cp "$test_file" "$test_backup"
    cp "$vm_lib" "$vm_backup"
    [[ ! -e "$probe" && ! -e "$conflict" && ! -e "$vm_probe" ]] || fail "async probe path already exists"
    cp "$lib" "$backup"
    trap 'cp "$backup" "$lib"; cp "$test_backup" "$test_file"; cp "$vm_backup" "$vm_lib"; rm -f "$probe" "$conflict" "$vm_probe" "$backup" "$test_backup" "$vm_backup"' EXIT

    log_step "Probe 4/4: async intrinsic scanner, selector and dispatch wiring"
    run_logged "$LOG_DIR/async-baseline.log" cargo build -p dotnet-vm --no-default-features
    local generated
    generated="$(find target/debug/build -path '*/out/intrinsics_dispatch.rs' -printf '%T@ %p\n' | sort -nr | head -1 | cut -d' ' -f2-)"
    sed -n '/pub fn dispatch_method_intrinsic/,/^}/p' "$generated" > "$LOG_DIR/async-baseline-arms"

    printf '\npub mod async_build_probe;\n' >> "$lib"
    printf '\nmod async_nested_probe;\n' >> "$vm_lib"
    cat > "$vm_probe" <<'EOF'
use crate::{StepResult, stack::ops::VesOps};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnetdll::prelude::MethodMemberIndex;

#[dotnet_intrinsic("static void DotnetRs.AsyncProbe::Nested()")]
pub fn nested<'gc, T: VesOps<'gc>>(ctx: &mut T, method: MethodDescription, generics: &GenericLookup) -> StepResult {
    let index = method.parent.definition().methods.iter().position(|m| m.name == "Unselected").unwrap();
    let target = MethodDescription::new(method.parent.clone(), method.parent_generics.clone(), method.resolution(), MethodMemberIndex::Method(index));
    ctx.dispatch_method(target, generics.clone())
}
EOF
    cat > "$probe" <<'EOF'
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_vm_ops::effects::{NativeInvocation, NativeOperandView, NativeResult, NativeSelection, VmEffects};
use crate::IntrinsicStringHost;

pub fn sync<'gc, T: IntrinsicStringHost<'gc>>(
    _ctx: &mut T, _method: MethodDescription, _generics: &GenericLookup,
) -> dotnet_vm_data::StepResult { dotnet_vm_data::StepResult::Continue }
pub fn choose<'gc, T: IntrinsicStringHost<'gc>>(
    _ctx: &T, _operands: NativeOperandView<'_, 'gc>, _method: &MethodDescription, _generics: &GenericLookup,
) -> NativeSelection<T> {
    NativeSelection::Async(NativeInvocation { start: 0, length: 0, parameter_count: 0 })
}
#[dotnet_intrinsic("static void DotnetRs.AsyncProbe::First()", select = self::choose, fallback = self::sync)]
#[dotnet_intrinsic("static void DotnetRs.AsyncProbe::Second(int)", select = self::choose, fallback = self::sync)]
pub async fn selected(mut fx: VmEffects) -> NativeResult { fx.yield_now().await?; Ok(()) }
#[dotnet_intrinsic("static void DotnetRs.AsyncProbe::Unselected()", fallback = self::sync)]
pub async fn unselected(mut fx: VmEffects) -> NativeResult { fx.yield_now().await?; Ok(()) }
EOF
    cat scripts/tests/async_dispatch_probe.rs >> "$test_file"
    for config in st mt; do
        local -a features=(--no-default-features)
        [[ "$config" == st ]] || features+=(--features multithreading)
        run_logged "$LOG_DIR/async-$config.log" cargo build -p dotnet-vm "${features[@]}"
        generated="$(find target/debug/build -path '*/out/intrinsics_dispatch.rs' -printf '%T@ %p\n' | sort -nr | head -1 | cut -d' ' -f2-)"
        rg -q 'Some\(dotnet_intrinsics_string::async_build_probe::selected_native_selector\)' "$generated" || fail "missing selected wiring ($config)"
        [[ "$(grep -c 'fn selector().*Some(dotnet_intrinsics_string::async_build_probe::selected_native_selector)' "$generated")" == 2 ]] || fail "expected two selector providers ($config)"
        [[ "$(grep -c 'fn selector().*{ None }' "$generated")" == 1 ]] || fail "expected one unselected provider ($config)"
        [[ "$(grep -c 'fn fallback().*async_build_probe::.*_native_fallback' "$generated")" == 3 ]] || fail "expected three fallback providers ($config)"
        [[ "$(grep -c '=> dotnet_intrinsics_string::async_build_probe::selected_native_entry' "$generated")" == 2 ]] || fail "expected two selected arms ($config)"
        rg -q 'unselected_native_entry::<T, NativeSelector_.*>\(ctx, method, generics, _mode\)' "$generated" || fail "missing unselected wiring ($config)"
        sed -n '/pub fn dispatch_method_intrinsic/,/^}/p' "$generated" | grep -v 'DotnetRs_AsyncProbe_' > "$LOG_DIR/async-$config-arms"
        cmp "$LOG_DIR/async-baseline-arms" "$LOG_DIR/async-$config-arms" || fail "existing dispatch arms changed ($config)"
        run_logged "$LOG_DIR/async-dispatch-$config.log" cargo nextest run -p dotnet-vm "${features[@]}" async_probe_dispatch_modes
        rg -q '1 test run: 1 passed' "$LOG_DIR/async-dispatch-$config.log" || fail "async dispatch probe did not pass exactly once ($config)"
    done

    # The unreferenced source is scanned by build.rs but never compiled as a module.
    cat > "$conflict" <<'EOF'
#[dotnet_intrinsic("static void DotnetRs.AsyncConflict::First()", select = one, fallback = sync)]
#[dotnet_intrinsic("static void DotnetRs.AsyncConflict::Second()", select = two, fallback = sync)]
async fn conflict(_fx: VmEffects) -> NativeResult { unreachable!() }
EOF
    for case in different missing; do
        if [[ "$case" == missing ]]; then
            sed -i 's/, select = two//' "$conflict"
        fi
        if cargo build -p dotnet-vm --no-default-features >"$LOG_DIR/async-conflict-$case.log" 2>&1; then
            fail "$case selector mismatch accepted by scanner"
        fi
        rg -q 'async_selector_conflict.rs: intrinsic annotations must name the same selector' "$LOG_DIR/async-conflict-$case.log" || { print_log_excerpt "$LOG_DIR/async-conflict-$case.log"; fail "missing scanner conflict diagnostic ($case)"; }
    done
    for case in different missing; do
        cat > "$conflict" <<'EOF'
#[dotnet_intrinsic("static void DotnetRs.AsyncConflict::First()", fallback = one)]
#[dotnet_intrinsic("static void DotnetRs.AsyncConflict::Second()", fallback = two)]
async fn conflict(_fx: VmEffects) -> NativeResult { unreachable!() }
EOF
        local diagnostic='intrinsic annotations must name the same fallback'
        if [[ "$case" == missing ]]; then
            sed -i 's/, fallback = two//' "$conflict"
            diagnostic='async intrinsic requires a fallback'
        fi
        if cargo build -p dotnet-vm --no-default-features >"$LOG_DIR/async-fallback-$case.log" 2>&1; then
            fail "$case fallback accepted by scanner"
        fi
        rg -q "$diagnostic" "$LOG_DIR/async-fallback-$case.log" || { print_log_excerpt "$LOG_DIR/async-fallback-$case.log"; fail "missing scanner fallback diagnostic ($case)"; }
    done
)

main() {
    require_cmd cargo
    require_cmd dotnet
    require_cmd rg
    require_cmd stat
    require_cmd find
    require_cmd tee

    assert_clippy_skips_dotnet
    assert_skip_toggle_invalidates_support_build
    assert_vm_directory_rerun_invalidation
    assert_async_intrinsic_wiring

    log_step "Build-script regression probes passed"
}

main "$@"
