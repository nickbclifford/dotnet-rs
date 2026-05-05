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

support_dll_path() {
    find target -path '*/build/dotnet-assemblies-*/out/support.dll' -type f | head -n1
}

assert_clippy_skips_dotnet() {
    local log_file="$LOG_DIR/clippy-clean.log"
    log_step "Probe 1/3: clean clippy should skip dotnet/MSBuild invocation paths"
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

    log_step "Probe 2/3: DOTNET_SKIP_BUILD toggle must invalidate dotnet-assemblies output"
    cargo clean
    DOTNET_SKIP_BUILD=1 cargo build -p dotnet-assemblies 2>&1 | tee "$skip_log"

    dll_path="$(support_dll_path)"
    [[ -n "$dll_path" ]] || fail "support.dll was not produced under skip mode"
    size="$(stat -c%s "$dll_path")"
    [[ "$size" == "0" ]] || fail "expected stub support.dll size 0 in skip mode, got $size at $dll_path"

    cargo build -p dotnet-assemblies -vv 2>&1 | tee "$noskip_log"
    rg -n 'env variable DOTNET_SKIP_BUILD changed' "$noskip_log" >/dev/null \
        || fail "missing DOTNET_SKIP_BUILD env invalidation marker in -vv log ($noskip_log)"

    dll_path="$(support_dll_path)"
    [[ -n "$dll_path" ]] || fail "support.dll missing after non-skip rebuild"
    size="$(stat -c%s "$dll_path")"
    [[ "$size" -gt 0 ]] || fail "expected non-empty support.dll after unsetting skip, got $size at $dll_path"
}

assert_vm_directory_rerun_invalidation() {
    local baseline_log="$LOG_DIR/vm-rerun-baseline.log"
    local add_log="$LOG_DIR/vm-rerun-add.log"
    local remove_log="$LOG_DIR/vm-rerun-remove.log"
    local probe_file="crates/dotnet-vm/src/intrinsics/__rerun_probe.rs"

    log_step "Probe 3/3: dotnet-vm rerun-if-changed should retrigger on add/remove under watched root"
    cargo clean
    cargo build -p dotnet-vm --no-default-features -vv >"$baseline_log" 2>&1

    rm -f "$probe_file"
    cat >"$probe_file" <<'EOF'
// Temporary probe file for build-script invalidation test.
EOF

    cargo build -p dotnet-vm --no-default-features -vv >"$add_log" 2>&1
    rg -n 'Dirty dotnet-vm .*src/intrinsics' "$add_log" >/dev/null \
        || fail "add-file rerun marker missing in $add_log"

    rm -f "$probe_file"
    cargo build -p dotnet-vm --no-default-features -vv >"$remove_log" 2>&1
    rg -n 'Dirty dotnet-vm .*src/intrinsics' "$remove_log" >/dev/null \
        || fail "remove-file rerun marker missing in $remove_log"
}

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

    log_step "Build-script regression probes passed"
}

main "$@"
