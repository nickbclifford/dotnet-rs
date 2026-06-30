#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SINGLE_FILE_CSPROJ="$ROOT_DIR/crates/dotnet-cli/tests/SingleFile.csproj"
TMP_DIFF_RUN_DIR=""

usage() {
    cat >&2 <<'EOF'
Usage:
  bash scripts/diff_run.sh [--host|--assemblies] <path-to-fixture.cs>
  bash scripts/diff_run.sh [--host|--assemblies] <path-to-project.csproj>
  bash scripts/diff_run.sh [--host|--assemblies] <path-to-prebuilt.dll>

Options:
  --host        Run dotnet-rs in host mode (no -a). This is the default.
  --assemblies  Run dotnet-rs in explicit `-a <shared_framework_dir>` mode.

Builds a C# fixture with crates/dotnet-cli/tests/SingleFile.csproj (for .cs input),
or builds the supplied project (for .csproj input), then runs the resulting DLL
under both `dotnet` and `dotnet-rs` and diffs exit code + stdout.

The default mode can also be set via DOTNET_RS_DIFF_MODE=host|assemblies.
EOF
}

fail() {
    echo "ERROR: $*" >&2
    exit 2
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

make_abs_path() {
    local path="$1"
    if [[ "$path" = /* ]]; then
        printf '%s\n' "$path"
    else
        printf '%s/%s\n' "$PWD" "$path"
    fi
}

latest_runtime_in_base() {
    local base="$1"
    [[ -d "$base" ]] || return 1

    local latest
    latest="$(find "$base" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | grep -E '^[0-9]' | sort -V | tail -n 1 || true)"
    [[ -n "$latest" ]] || return 1

    printf '%s/%s\n' "$base" "$latest"
}

find_shared_framework_dir() {
    local candidate

    if [[ -n "${DOTNET_ROOT:-}" ]]; then
        candidate="$(latest_runtime_in_base "$DOTNET_ROOT/shared/Microsoft.NETCore.App" || true)"
        if [[ -n "$candidate" ]]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    fi

    candidate="$(
        dotnet --list-runtimes 2>/dev/null \
            | awk '/^Microsoft\.NETCore\.App / { gsub(/[\[\]]/, "", $3); print $2 "|" $3 }' \
            | sort -t '|' -k1,1V \
            | tail -n 1 \
            | awk -F'|' '{ print $2 "/" $1 }'
    )"
    if [[ -n "$candidate" && -d "$candidate" ]]; then
        printf '%s\n' "$candidate"
        return 0
    fi

    local bases=()
    case "$(uname -s)" in
        Darwin)
            bases=("/usr/local/share/dotnet/shared/Microsoft.NETCore.App")
            ;;
        Linux)
            bases=(
                "/usr/share/dotnet/shared/Microsoft.NETCore.App"
                "/usr/lib/dotnet/shared/Microsoft.NETCore.App"
            )
            ;;
        *)
            bases=()
            ;;
    esac

    local base
    for base in "${bases[@]}"; do
        candidate="$(latest_runtime_in_base "$base" || true)"
        if [[ -n "$candidate" ]]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done

    return 1
}

resolve_dotnet_rs_runner() {
    if [[ -n "${DOTNET_RS_BIN:-}" ]]; then
        [[ -x "$DOTNET_RS_BIN" ]] || fail "DOTNET_RS_BIN is not executable: $DOTNET_RS_BIN"
        printf '%s\n' "$DOTNET_RS_BIN"
        return 0
    fi

    if [[ -x "$ROOT_DIR/target/debug/dotnet-rs" ]]; then
        printf '%s\n' "$ROOT_DIR/target/debug/dotnet-rs"
        return 0
    fi

    if command -v dotnet-rs >/dev/null 2>&1; then
        command -v dotnet-rs
        return 0
    fi

    return 1
}

run_capture_stdout_and_exit() {
    local stdout_file="$1"
    local exit_file="$2"
    shift 2

    set +e
    "$@" >"$stdout_file"
    local exit_code=$?
    set -e

    printf '%s\n' "$exit_code" >"$exit_file"
}

write_report() {
    local report_file="$1"
    local exit_file="$2"
    local stdout_file="$3"

    {
        printf 'exit_code=%s\n' "$(<"$exit_file")"
        printf 'stdout:\n'
        cat "$stdout_file"
    } >"$report_file"
}

main() {
    local mode="${DOTNET_RS_DIFF_MODE:-host}"
    case "$mode" in
        host|assemblies)
            ;;
        *)
            fail "DOTNET_RS_DIFF_MODE must be 'host' or 'assemblies' (got: $mode)"
            ;;
    esac

    while [[ $# -gt 0 ]]; do
        case "$1" in
            --host)
                mode="host"
                shift
                ;;
            --assemblies)
                mode="assemblies"
                shift
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            --)
                shift
                break
                ;;
            -*)
                fail "unknown option: $1"
                ;;
            *)
                break
                ;;
        esac
    done

    [[ $# -eq 1 ]] || {
        usage
        exit 2
    }

    require_cmd dotnet
    require_cmd diff
    require_cmd awk
    require_cmd find
    require_cmd sort

    local input_path
    input_path="$(make_abs_path "$1")"
    [[ -f "$input_path" ]] || fail "input not found: $1"

    TMP_DIFF_RUN_DIR="$(mktemp -d "${TMPDIR:-/tmp}/dotnet-rs-diff-run.XXXXXX")"
    trap '[[ -n "${TMP_DIFF_RUN_DIR:-}" ]] && rm -rf "$TMP_DIFF_RUN_DIR"' EXIT

    local dll_path
    case "$input_path" in
        *.cs)
            [[ -f "$SINGLE_FILE_CSPROJ" ]] || fail "SingleFile.csproj not found at $SINGLE_FILE_CSPROJ"
            local build_out
            build_out="$TMP_DIFF_RUN_DIR/build"
            mkdir -p "$build_out"

            dotnet build "$SINGLE_FILE_CSPROJ" \
                -p:AllowUnsafeBlocks=true \
                "-p:TestFile=$input_path" \
                -o "$build_out" \
                "-p:BaseIntermediateOutputPath=$build_out/obj/" \
                -v:q \
                --nologo \
                >/dev/null

            dll_path="$build_out/SingleFile.dll"
            [[ -f "$dll_path" ]] || fail "expected build output missing: $dll_path"
            ;;
        *.csproj)
            local build_out
            build_out="$TMP_DIFF_RUN_DIR/build"
            mkdir -p "$build_out"

            dotnet build "$input_path" \
                -o "$build_out" \
                "-p:BaseIntermediateOutputPath=$build_out/obj/" \
                -v:q \
                --nologo \
                >/dev/null

            local project_name
            project_name="$(basename "$input_path" .csproj)"
            dll_path="$build_out/$project_name.dll"
            if [[ ! -f "$dll_path" ]]; then
                local dll_count
                dll_count="$(find "$build_out" -maxdepth 1 -type f -name '*.dll' | wc -l | tr -d ' ')"
                if [[ "$dll_count" -eq 1 ]]; then
                    dll_path="$(find "$build_out" -maxdepth 1 -type f -name '*.dll' -print -quit)"
                else
                    fail "could not identify project entry DLL in $build_out; expected $project_name.dll or exactly one DLL"
                fi
            fi
            ;;
        *.dll)
            dll_path="$input_path"
            ;;
        *)
            fail "input must be a .cs fixture, a .csproj project, or a .dll: $1"
            ;;
    esac

    local shared_framework_dir=""
    if [[ "$mode" == "assemblies" ]]; then
        shared_framework_dir="$(find_shared_framework_dir || true)"
        [[ -n "$shared_framework_dir" ]] || fail "could not locate Microsoft.NETCore.App shared framework directory"
    fi

    local dotnet_rs_bin=""
    local use_cargo_run=0
    # NB: do not wrap resolve_dotnet_rs_runner in `|| true` — that masks its
    # non-zero "not found" exit so the `if` always takes the success branch and
    # we end up invoking an empty command. Check the exit status and the result.
    if ! dotnet_rs_bin="$(resolve_dotnet_rs_runner)" || [[ -z "$dotnet_rs_bin" ]]; then
        use_cargo_run=1
        dotnet_rs_bin=""
    fi

    local dotnet_stdout="$TMP_DIFF_RUN_DIR/dotnet.stdout"
    local dotnet_exit="$TMP_DIFF_RUN_DIR/dotnet.exit"
    local rs_stdout="$TMP_DIFF_RUN_DIR/dotnet-rs.stdout"
    local rs_exit="$TMP_DIFF_RUN_DIR/dotnet-rs.exit"

    run_capture_stdout_and_exit "$dotnet_stdout" "$dotnet_exit" dotnet "$dll_path"

    if [[ "$use_cargo_run" -eq 1 ]]; then
        if [[ "$mode" == "assemblies" ]]; then
            (cd "$ROOT_DIR" && run_capture_stdout_and_exit "$rs_stdout" "$rs_exit" cargo run --quiet --bin dotnet-rs --no-default-features -- -a "$shared_framework_dir" "$dll_path")
        else
            (cd "$ROOT_DIR" && run_capture_stdout_and_exit "$rs_stdout" "$rs_exit" cargo run --quiet --bin dotnet-rs --no-default-features -- "$dll_path")
        fi
    else
        if [[ "$mode" == "assemblies" ]]; then
            run_capture_stdout_and_exit "$rs_stdout" "$rs_exit" "$dotnet_rs_bin" -a "$shared_framework_dir" "$dll_path"
        else
            run_capture_stdout_and_exit "$rs_stdout" "$rs_exit" "$dotnet_rs_bin" "$dll_path"
        fi
    fi

    local dotnet_report="$TMP_DIFF_RUN_DIR/dotnet.report"
    local rs_report="$TMP_DIFF_RUN_DIR/dotnet-rs.report"
    write_report "$dotnet_report" "$dotnet_exit" "$dotnet_stdout"
    write_report "$rs_report" "$rs_exit" "$rs_stdout"

    if diff -u "$dotnet_report" "$rs_report" >/dev/null; then
        echo "PASS: exit code and stdout match (mode=$mode)"
        return 0
    fi

    echo "FAIL: outputs diverged (dotnet vs dotnet-rs, mode=$mode)" >&2
    diff -u "$dotnet_report" "$rs_report" >&2 || true
    return 1
}

main "$@"
