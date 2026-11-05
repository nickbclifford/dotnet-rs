#!/usr/bin/env bash
# Bounded pointer diagnostics and control gates. No probe bodies live here.
set -uo pipefail

# Explicit control identities, singleton shard and batch assignments; edit alongside each
# promotion. Row format: name|shard|batch. A batch runs its controls in row order on one thread
# (batches::batch_N in pointer_provenance_tests.rs); `verify-batches` fails if the Rust batch
# lists drift from this table.
expected_controls=(
'pointer_provenance_tests::p00_smoke|1|1'
'pointer_provenance_tests::p02_ldloca_stind_i4_control|2|2'
'pointer_provenance_tests::p03_ldloca_convu_ldind_i4|3|3'
'pointer_provenance_tests::p05_single_localloc_roundtrip|4|4'
'pointer_provenance_tests::p07_localloc_one_past_end|5|1'
'pointer_provenance_tests::p11_ldelema_stind_heap_array_control|6|2'
'pointer_provenance_tests::p10_ldelema_convu_stind_heap_array|7|3'
'pointer_provenance_tests::p20_heap_convu_then_typed_write_then_raw_store|8|4'
'pointer_provenance_tests::p13_ldelema_managed_add_far_out_of_bounds_then_conv_u|9|1'
'pointer_provenance_tests::p14_initblk_heap_array_via_managed_ptr|10|2'
'pointer_provenance_tests::p16_cpblk_heap_arrays_via_managed_ptr|11|3'
'pointer_provenance_tests::p_null_through_pointer_local_address|12|4'
'pointer_provenance_tests::p06_localloc_minus_one_then_plus_one|13|1'
'pointer_provenance_tests::p08_localloc_overshoot|14|2'
'pointer_provenance_tests::p17_localloc_ptr_ceq_self|15|3'
'pointer_provenance_tests::p18_localloc_ptr_clt|16|4'
'pointer_provenance_tests::p21_localloc_signed_extreme_offsets_roundtrip|17|1'
'pointer_provenance_tests::p22_localloc_wrap_to_zero_roundtrip|18|2'
'pointer_provenance_tests::p23_localloc_pointer_loop_unsigned_branch_compare|19|3'
'pointer_provenance_tests::p24_unsafe_byte_offset_direction_and_deref|20|4'
'pointer_provenance_tests::p25_convi_and_stind_i_pointer_roundtrip_deref|21|1'
'pointer_provenance_tests::p26_native_int_boundary_store_load_roundtrip|22|2'
'pointer_provenance_tests::p27_stind_i_fresh_pointer_through_local_slot_dereferences|23|3'
'pointer_provenance_tests::p28_stind_i_fresh_pointer_through_heap_memory_dereferences|24|4'
'pointer_provenance_tests::p04_double_localloc_write_first|25|1'
'pointer_provenance_tests::p29_localloc_blocks_survive_growth_and_are_16_byte_aligned|26|2'
'pointer_provenance_tests::p30_localloc_block_survives_nested_callee_localloc|27|3'
'pointer_provenance_tests::p31_localloc_blocks_released_on_return_and_pooling|28|4'
'pointer_provenance_tests::p32_localloc_blocks_released_on_tail_call_replacement|29|1'
'pointer_provenance_tests::p33_localloc_blocks_released_on_exception_unwind|30|2'
'pointer_provenance_tests::p34_localloc_blocks_released_on_executor_teardown|31|3'
'pointer_provenance_tests::p35_ldflda_on_stack_value_type_then_ldind|32|4'
'pointer_provenance_tests::p36_static_address_survives_forced_collection|33|1'
'pointer_provenance_tests::p01_ldloca_convu_stind_i4|34|2'
'pointer_provenance_tests::p37_ldloca_value_type_survives_stloc|35|3'
)

# full name | classification | native diagnostic permission | control shard
catalog=(
'pointer_provenance_tests::p00_smoke|control|no|1'
'pointer_provenance_tests::p01_ldloca_convu_stind_i4|control|no|34'
'pointer_provenance_tests::p02_ldloca_stind_i4_control|control|no|2'
'pointer_provenance_tests::p03_ldloca_convu_ldind_i4|control|no|3'
'pointer_provenance_tests::p04_double_localloc_write_first|control|no|25'
'pointer_provenance_tests::p05_single_localloc_roundtrip|control|no|4'
'pointer_provenance_tests::p06_localloc_minus_one_then_plus_one|control|no|13'
'pointer_provenance_tests::p07_localloc_one_past_end|control|no|5'
'pointer_provenance_tests::p08_localloc_overshoot|control|no|14'
'pointer_provenance_tests::p09_ldloca_convu_survives_callee_stack_growth|reported-ub|no|-'
'pointer_provenance_tests::p10_ldelema_convu_stind_heap_array|control|no|7'
'pointer_provenance_tests::p11_ldelema_stind_heap_array_control|control|no|6'
'pointer_provenance_tests::p12_ldsflda_convu_stind_static|suspected-ub|no|-'
'pointer_provenance_tests::p13_ldelema_managed_add_far_out_of_bounds_then_conv_u|control|no|9'
'pointer_provenance_tests::p14_initblk_heap_array_via_managed_ptr|control|no|10'
'pointer_provenance_tests::p15_initblk_stack_local_via_managed_ptr|suspected-ub|no|-'
'pointer_provenance_tests::p16_cpblk_heap_arrays_via_managed_ptr|control|no|11'
'pointer_provenance_tests::p_null_through_pointer_local_address|control|no|12'
'pointer_provenance_tests::p17_localloc_ptr_ceq_self|control|no|15'
'pointer_provenance_tests::p18_localloc_ptr_clt|control|no|16'
'pointer_provenance_tests::p19_ldelem_negative_index|panic|yes|-'
'pointer_provenance_tests::p20_heap_convu_then_typed_write_then_raw_store|control|no|8'
'pointer_provenance_tests::p21_localloc_signed_extreme_offsets_roundtrip|control|no|17'
'pointer_provenance_tests::p22_localloc_wrap_to_zero_roundtrip|control|no|18'
'pointer_provenance_tests::p23_localloc_pointer_loop_unsigned_branch_compare|control|no|19'
'pointer_provenance_tests::p24_unsafe_byte_offset_direction_and_deref|control|no|20'
'pointer_provenance_tests::p25_convi_and_stind_i_pointer_roundtrip_deref|control|no|21'
'pointer_provenance_tests::p26_native_int_boundary_store_load_roundtrip|control|no|22'
'pointer_provenance_tests::p27_stind_i_fresh_pointer_through_local_slot_dereferences|control|no|23'
'pointer_provenance_tests::p28_stind_i_fresh_pointer_through_heap_memory_dereferences|control|no|24'
'pointer_provenance_tests::p29_localloc_blocks_survive_growth_and_are_16_byte_aligned|control|no|26'
'pointer_provenance_tests::p30_localloc_block_survives_nested_callee_localloc|control|no|27'
'pointer_provenance_tests::p31_localloc_blocks_released_on_return_and_pooling|control|no|28'
'pointer_provenance_tests::p32_localloc_blocks_released_on_tail_call_replacement|control|no|29'
'pointer_provenance_tests::p33_localloc_blocks_released_on_exception_unwind|control|no|30'
'pointer_provenance_tests::p34_localloc_blocks_released_on_executor_teardown|control|no|31'
'pointer_provenance_tests::p35_ldflda_on_stack_value_type_then_ldind|control|no|32'
'pointer_provenance_tests::p36_static_address_survives_forced_collection|control|no|33'
'pointer_provenance_tests::p37_ldloca_value_type_survives_stloc|control|no|35'
'pointer_provenance_tests::p_wide_store_into_int_local|unverified|no|-'
)
flags='-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks'
die() { echo "error: $*" >&2; exit 2; }
# Refuse stale, duplicate or non-singleton partitions before discovery.
reviewed_count=35
declare -A expected_by_name=() batch_by_name=() seen_names=() seen_controls=() shard_counts=() batch_members=()
shards=() batches=()
for row in "${expected_controls[@]}"; do
    IFS='|' read -r name group batch extra <<< "$row"
    [[ $name == pointer_provenance_tests::p* && $group =~ ^[1-9][0-9]*$ && $batch =~ ^[1-9][0-9]*$ && -z ${extra:-} && ! -v expected_by_name[$name] ]] || die 'invalid or duplicate expected control'
    [[ $group == $((${#shards[@]} + 1)) ]] || die 'expected singleton shards must be sequential'
    expected_by_name[$name]=$group
    batch_by_name[$name]=$batch
    batch_members[$batch]+="${batch_members[$batch]:+ }$name"
    shards+=("$group")
done
for ((b = 1; b <= ${#batch_members[@]}; b++)); do
    [[ -v batch_members[$b] ]] || die 'expected batches must be sequential'
    batches+=("$b")
done
((${#batches[@]} > 0)) || die 'expected control table must define batches'
((${#expected_controls[@]} == reviewed_count)) || die 'expected control count differs from reviewed count'
for row in "${catalog[@]}"; do
    IFS='|' read -r name class native group extra <<< "$row"
    [[ $name == pointer_provenance_tests::p* && ! -v seen_names[$name] && -z ${extra:-} ]] || die "invalid or duplicate catalog identity: $name"
    seen_names[$name]=1
    if [[ $class != control ]]; then
        [[ $group == - ]] || die "diagnostic assigned to control shard: $name"
        continue
    fi
    [[ -v expected_by_name[$name] && ${expected_by_name[$name]} == "$group" && ! -v seen_controls[$name] ]] || die "unexpected, duplicate or mis-sharded control: $name"
    seen_controls[$name]=1
    shard_counts[$group]=$(( ${shard_counts[$group]:-0} + 1 ))
    ((shard_counts[$group] == 1)) || die "non-singleton control shard: $group"
done
((${#seen_controls[@]} == reviewed_count && ${#shard_counts[@]} == reviewed_count)) || die 'catalog partition differs from reviewed controls'
shard_options=$(IFS='|'; echo "${shards[*]}")
batch_options=$(IFS='|'; echo "${batches[*]}")
usage() {
    echo "Usage: pointer_provenance.sh list | verify-batches | verify-controls --configuration no-features|multithreading [--dry-run] | controls --configuration CONFIG --shard $shard_options [--dry-run] | batch --configuration CONFIG --batch $batch_options [--dry-run] | diagnostic --configuration CONFIG (--probe SHORT_NAME | --all) [--lane permissive-tb|native] [--dry-run]"
    echo 'diagnostic --all selects diagnostic-only probes; enabled controls use controls. Native diagnostics require an individually permitted probe.'
}
command=${1:-}; [[ -n $command ]] || { usage >&2; exit 2; }; shift
config= shard= batch= probe= lane=permissive-tb all=0 dry=0
while (($#)); do
    case $1 in
        --configuration|--shard|--batch|--probe|--lane)
            (($# >= 2)) || die "missing value for $1"
            case $1 in
                --configuration) [[ -z $config ]] || die 'duplicate configuration'; config=$2 ;;
                --shard) [[ -z $shard ]] || die 'duplicate shard'; shard=$2 ;;
                --batch) [[ -z $batch ]] || die 'duplicate batch'; batch=$2 ;;
                --probe) [[ -z $probe ]] || die 'duplicate probe'; probe=$2 ;;
                --lane) [[ ${lane_set:-0} == 0 ]] || die 'duplicate lane'; lane=$2; lane_set=1 ;;
            esac
            shift 2 ;;
        --all) ((all == 0)) || die 'duplicate --all'; all=1; shift ;;
        --dry-run) ((dry == 0)) || die 'duplicate --dry-run'; dry=1; shift ;;
        *) die "unknown argument $1" ;;
    esac
done
[[ $lane == permissive-tb || $lane == native ]] || die 'unknown lane'
if [[ $command == list ]]; then
    [[ -z $config && -z $shard && -z $batch && -z $probe && $all == 0 && ${lane_set:-0} == 0 ]] || die 'list takes no options except --dry-run'
    printf '%s\n' 'full-name | classification | native-diagnostic | shard' "${catalog[@]}"
    exit 0
fi
if [[ $command == verify-batches ]]; then
    [[ -z $config && -z $shard && -z $batch && -z $probe && $all == 0 && $dry == 0 && ${lane_set:-0} == 0 ]] || die 'verify-batches takes no options'
fi
[[ $command == verify-batches || $command == verify-controls || $command == controls || $command == batch || $command == diagnostic ]] || die 'unknown command'
[[ $command == verify-batches ]] || [[ $config == no-features || $config == multithreading ]] || die 'configuration required: no-features or multithreading'
features=(--no-default-features)
[[ $config != multithreading ]] || features+=(--features multithreading)
selected=()
for row in "${catalog[@]}"; do
    IFS='|' read -r name class native group <<< "$row"
    case $command in
        verify-controls) [[ $class != control ]] || selected+=("$name") ;;
        controls) [[ $class != control || $group != "$shard" ]] || selected+=("$name") ;;
        batch|verify-batches) ;;
        diagnostic)
            if ((all)) || [[ -n $probe && ${name##*::} == "$probe" ]]; then
                if [[ $class == control ]]; then
                    ((all)) && continue
                    die "$name is an enabled control; use controls --shard $group instead"
                fi
                [[ $lane != native || $native == yes ]] || die "native diagnostic not permitted for $name"
                selected+=("$name")
            fi ;;
    esac
done
case $command in
    verify-controls) [[ -z $shard && -z $batch && -z $probe && $all == 0 && ${lane_set:-0} == 0 ]] || die 'invalid verify-controls options'; ((${#selected[@]} > 0)) || die 'catalog must contain controls' ;;
    verify-batches) ;;
    controls) [[ -z $batch ]] || die 'controls does not accept batch'; [[ $shard =~ ^[1-9][0-9]*$ && -v shard_counts[$shard] ]] || die "shard required: $shard_options"; [[ -z $probe && $all == 0 && ${lane_set:-0} == 0 ]] || die 'invalid controls options'; ((${#selected[@]} == shard_counts[$shard] && ${#selected[@]} > 0)) || die 'catalog shard control count mismatch' ;;
    batch)
        [[ $batch =~ ^[1-9][0-9]*$ && -v batch_members[$batch] ]] || die "batch required: $batch_options"
        [[ -z $shard && -z $probe && $all == 0 && ${lane_set:-0} == 0 ]] || die 'invalid batch options'
        read -r -a batch_expected <<< "${batch_members[$batch]}"
        selected=("pointer_provenance_tests::batches::batch_$batch") ;;
    diagnostic) [[ -z $shard && -z $batch ]] || die 'diagnostic does not accept shard or batch'; if ((all)); then [[ -z $probe ]] || die 'choose --all or --probe'; else [[ -n $probe ]] && ((${#selected[@]} == 1)) || die 'unknown or missing exact short probe name'; fi ;;
esac
show() { printf '+'; printf ' %q' "$@"; printf '\n'; }
list_cmd=(cargo test -p dotnet-vm --lib "${features[@]}" -- --list)
metadata_cmd=(cargo nextest list -p dotnet-vm --lib "${features[@]}" -E 'test(pointer_provenance_tests)' --message-format json)
verify_controls() {
    local dir
    show "${list_cmd[@]}"; show "${metadata_cmd[@]}"
    ((dry)) && return 0
    dir=$(mktemp -d) || return 1
    if ! "${list_cmd[@]}" > "$dir/list" || ! "${metadata_cmd[@]}" > "$dir/metadata"; then
        echo 'control discovery failed' >&2; rm -rf "$dir"; return 1
    fi
    # The probe lane is the p-prefixed namespace; other tests in this module are
    # separate regressions, not diagnostic probes or members of these shards.
    if ! python3 - "$dir/list" "$dir/metadata" "${catalog[@]}" <<'PYCODE'
import json, sys
from collections import Counter
from pathlib import Path

prefix = "pointer_provenance_tests::p"
listing = Counter(line.removesuffix(": test") for line in Path(sys.argv[1]).read_text().splitlines()
                  if line.startswith(prefix) and line.endswith(": test"))
try:
    rows = [row.split("|") for row in sys.argv[3:]]
    controls = {name for name, cls, _, _ in rows if cls == "control"}
    diagnostics = {name for name, cls, _, _ in rows if cls != "control"}
    suites = json.loads(Path(sys.argv[2]).read_text())["rust-suites"]
    cases = {}
    for suite in suites.values():
        if suite.get("package-name") != "dotnet-vm" or suite.get("kind") != "lib" or suite.get("status") != "listed":
            continue
        for name, case in suite.get("testcases", {}).items():
            if name.startswith(prefix):
                if name in cases:
                    raise ValueError(f"duplicate metadata identity: {name}")
                cases[name] = case
    if set(listing) != set(cases) or any(count != 1 for count in listing.values()):
        raise ValueError("pointer probe listing and metadata differ or contain duplicates")
    for name, case in cases.items():
        ignored = case.get("ignored")
        match = case.get("filter-match", {})
        if ignored is False and match.get("status") != "matches":
            raise ValueError(f"unlisted or malformed pointer probe metadata: {name}")
        if ignored is True and (match.get("status") != "mismatch" or match.get("reason") != "ignored"):
            raise ValueError(f"malformed ignored pointer probe metadata: {name}")
        if ignored is not True and ignored is not False:
            raise ValueError(f"missing ignored status for pointer probe: {name}")
    for name in controls:
        case = cases.get(name, {})
        if listing[name] != 1 or case.get("ignored") is not False or case.get("filter-match", {}).get("status") != "matches":
            raise ValueError(f"{name}: list count={listing[name]}, metadata={case}")
    for name, case in cases.items():
        if case.get("ignored") is False and case.get("filter-match", {}).get("status") == "matches" and name not in diagnostics:
            if name not in controls or listing[name] != 1:
                raise ValueError(f"unmatched enabled pointer probe: {name}")
    if len(controls) != 35:
        raise ValueError(f"reviewed control count mismatch: {len(controls)}")
except (KeyError, TypeError, ValueError) as exc:
    print(f"control discovery/ignore guard failed: {exc}", file=sys.stderr)
    sys.exit(1)
print(f"control discovery: {len(controls)} exact non-ignored controls; 35 singleton shards partition enabled probes")
PYCODE
    then rm -rf "$dir"; return 1; fi
    rm -rf "$dir"
}
# Static drift guard (no Cargo): the Rust `control_batch!` invocations must list exactly the
# reviewed controls, each once, in the order and batch given by `expected_controls`; no
# diagnostic probe may appear; batch tests must live outside the `::p` discovery namespace.
verify_batches() {
    local src=${POINTER_PROVENANCE_SOURCE:-$script_dir/../crates/dotnet-vm/src/pointer_provenance_tests.rs}
    local -a table=() diag=()
    local row name class
    for row in "${expected_controls[@]}"; do table+=("$row"); done
    for row in "${catalog[@]}"; do
        IFS='|' read -r name class _ _ <<< "$row"
        [[ $class == control ]] || diag+=("${name##*::}")
    done
    python3 - "$src" "${#diag[@]}" "${diag[@]}" "${table[@]}" <<'PYCODE' || return 1
import re, sys
from pathlib import Path

try:
    text = Path(sys.argv[1]).read_text()
    n_diag = int(sys.argv[2])
    diagnostics = set(sys.argv[3:3 + n_diag])
    expected = {}
    order = []
    for row in sys.argv[3 + n_diag:]:
        name, _shard, batch = row.split("|")
        short = name.split("::")[-1]
        expected.setdefault(int(batch), []).append(short)
        order.append(short)
    mod = re.search(r"^mod batches \{(?P<body>.*)^\}", text, re.S | re.M)
    if mod is None:
        raise ValueError("pointer_provenance_tests.rs has no `mod batches` block")
    body = re.sub(r"//[^\n]*", "", mod.group("body"))
    body = body[body.index("macro_rules! control_batch"):]
    body = body[body.index("\n    }\n") + 1:]  # skip the macro definition
    found = {}
    for m in re.finditer(r"control_batch!\(\s*batch_(\d+)\s*:\s*\[(.*?)\]\s*\);", body, re.S):
        number = int(m.group(1))
        if number in found:
            raise ValueError(f"batch_{number} defined twice")
        entries = re.sub(r"#\[[^\n]*\]", "", m.group(2))
        found[number] = [e.strip() for e in entries.split(",") if e.strip()]
    if body.count("control_batch!(") != len(found):
        raise ValueError("unparsed control_batch! invocation")
    if sorted(found) != sorted(expected):
        raise ValueError(f"batch numbers differ: source {sorted(found)}, table {sorted(expected)}")
    flat = [n for b in sorted(found) for n in found[b]]
    bad = [n for n in flat if n in diagnostics]
    if bad:
        raise ValueError(f"diagnostic probe in a batch: {bad}")
    if len(flat) != len(set(flat)):
        raise ValueError(f"control in more than one batch: {sorted(n for n in set(flat) if flat.count(n) > 1)}")
    if set(flat) != set(order):
        raise ValueError(f"batched controls differ from reviewed controls: missing {sorted(set(order) - set(flat))}, extra {sorted(set(flat) - set(order))}")
    for number in sorted(expected):
        if found[number] != expected[number]:
            raise ValueError(f"batch_{number} differs from the table: source {found[number]}, table {expected[number]}")
except (ValueError, KeyError, IndexError) as exc:
    print(f"batch drift guard failed: {exc}", file=sys.stderr)
    sys.exit(1)
print(f"batch drift guard: {len(order)} reviewed controls in {len(expected)} batches, each exactly once, no diagnostics")
PYCODE
}
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
if [[ $command == verify-batches ]]; then
    verify_batches || exit 1
    exit 0
fi
if [[ $command == batch ]]; then
    verify_batches || exit 1
fi
if [[ $command == verify-controls || $command == controls || $command == batch ]]; then
    verify_controls || exit 1
    [[ $command != verify-controls ]] || exit 0
fi

# Miri may print a warning after libtest starts a test but before it prints the
# test status. In that case libtest emits the final `ok` on its own line.
recognized_single_pass() {
    python3 - "$1" "$2" <<'PY'
import re
import sys
from pathlib import Path

name, logfile = sys.argv[1:]
lines = Path(logfile).read_text(errors="replace").splitlines()
test_prefix = f"test {name} ... "
start = next((i for i, line in enumerate(lines) if line.startswith(test_prefix)), None)
summaries = [i for i, line in enumerate(lines)
             if re.match(r"^test result: ok\. 1 passed; 0 failed; 0 ignored;", line)]
if start is None or len(summaries) != 1 or summaries[0] <= start:
    sys.exit(1)
summary = summaries[0]
has_status = any(line == test_prefix + "ok" or line.strip() == "ok"
                 for line in lines[start:summary])
sys.exit(0 if has_status else 1)
PY
}

# A batch is one test that runs its controls sequentially and prints one
# `batch-control ok <full name>` line per completed control (and a `start` line before each,
# so a failure or timeout can name the control in flight). Pass only when the libtest summary is
# exactly 1 passed/0 failed/0 ignored and the ok lines equal the expected controls in order.
# Run with --nocapture so progress is visible even when the process is killed.
recognized_batch_pass() {
    python3 - "$1" "$2" "${@:3}" <<'PY'
import re
import sys
from pathlib import Path

name, logfile, *expected = sys.argv[1:]
lines = Path(logfile).read_text(errors="replace").splitlines()
done, started = [], []
for line in lines:
    m = re.search(r"batch-control (start|ok) (\S+)\s*$", line)
    if m:
        (started if m.group(1) == "start" else done).append(m.group(2))
in_flight = started[len(done)] if len(started) > len(done) else None
problems = []
if done != expected:
    problems.append(f"completed {len(done)} of {len(expected)} controls; "
                    f"missing {[n for n in expected if n not in done]}, "
                    f"unexpected {[n for n in done if n not in expected]}"
                    + ("" if sorted(done) != sorted(expected) or done == expected else ", order differs"))
if in_flight:
    problems.append(f"control in flight when the batch stopped: {in_flight}")
elif done and done != expected:
    problems.append(f"last completed control: {done[-1]}")
summaries = [i for i, line in enumerate(lines)
             if re.match(r"^test result: ok\. 1 passed; 0 failed; 0 ignored;", line)]
prefix = f"test {name} ... "
start = next((i for i, line in enumerate(lines) if line.startswith(prefix)), None)
if start is None or len(summaries) != 1 or summaries[0] <= start:
    problems.append("libtest summary is not exactly '1 passed; 0 failed; 0 ignored' for the batch test")
if problems:
    print(f"batch guard failed for {name}: " + "; ".join(problems), file=sys.stderr)
    sys.exit(1)
PY
}

# A bounded process per exact selector; retain complete output for diagnosis.
# Controls use the same count guard but never the ignored-test option.
if ((!dry)); then
    if [[ -n ${POINTER_PROVENANCE_LOG_DIR:-} ]]; then
        logdir=$POINTER_PROVENANCE_LOG_DIR
        mkdir -p -- "$logdir" || exit 1
    else
        logdir=$(mktemp -d "${TMPDIR:-/tmp}/pointer-provenance.XXXXXXXX") || exit 1
    fi
    echo "output directory: $logdir"
fi
# Per-process budget in seconds. A batch runs 8-9 controls (about 540 s locally),
# so it gets more than a singleton; ci.yml wraps each in a matching ci_diagnostics.py timeout.
proc_timeout=2700
[[ $command != batch ]] || proc_timeout=3000
failures=0; passes=0
summaries=()
for name in "${selected[@]}"; do
    if [[ $command == controls || $command == batch || $lane == permissive-tb ]]; then
        cmd=(env "MIRIFLAGS=$flags" cargo +nightly-2026-05-27 miri test -p dotnet-vm --lib "${features[@]}" -- --exact "$name" --test-threads=1)
    else
        cmd=(cargo test -p dotnet-vm --lib "${features[@]}" -- --exact "$name" --test-threads=1)
    fi
    if [[ $command == diagnostic ]]; then cmd+=(--ignored); fi
    if [[ $command == batch ]]; then cmd+=(--nocapture); fi
    show timeout -k 10s "${proc_timeout}s" "${cmd[@]}"
    ((dry)) && continue
    start=$(date +%s)
    logfile="$logdir/${name##*::}.log"
    timeout -k 10s "${proc_timeout}s" "${cmd[@]}" 2>&1 | tee "$logfile"
    statuses=("${PIPESTATUS[@]}")
    rc=${statuses[0]}; elapsed=$(( $(date +%s) - start ))
    if ((statuses[1] != 0 && rc == 0)); then rc=${statuses[1]}; fi
    if [[ $command == batch ]]; then
        recognized_batch_pass "$name" "$logfile" "${batch_expected[@]}" 2>"$logfile.guard"; guard_rc=$?
        guard_msg=$(<"$logfile.guard"); cat "$logfile.guard" >&2; cat "$logfile.guard" >> "$logfile"; rm -f "$logfile.guard"
    else
        guard_msg=; recognized_single_pass "$name" "$logfile"; guard_rc=$?
    fi
    if ((rc == 0 && guard_rc == 0)); then
        outcome=pass; ((passes+=1))
    else
        ((failures+=1))
        if ((rc == 124 || rc == 137)); then outcome=timeout-or-killed
        elif grep -Eqi 'error: Undefined Behavior|Miri.*undefined behavior' "$logfile"; then outcome=miri-ub
        elif grep -Eqi 'panicked at|assertion .*failed|test .* \.\.\. FAILED' "$logfile"; then outcome=assertion-or-panic
        elif ((rc == 0)); then outcome=zero-ignored-or-unverified-count
        else outcome=compile-setup-or-process-failure; fi
    fi
    summaries+=("$name: $outcome (exit=$rc, wall=${elapsed}s, log=$logfile)")
    if [[ $command == batch && $outcome != pass && -n $guard_msg ]]; then summaries+=("  $guard_msg"); fi
done
if ((!dry)); then
    printf '%s\n' '--- per-probe summary ---' "${summaries[@]}"
    echo "total: $passes passed, $failures failed"
    ((failures == 0)) || exit 1
    if [[ $command == controls ]] && ((passes != shard_counts[$shard])); then echo "expected exactly ${shard_counts[$shard]} passing controls" >&2; exit 1; fi
    if [[ $command == batch ]] && ((passes != 1)); then echo "expected exactly one passing batch" >&2; exit 1; fi
fi
