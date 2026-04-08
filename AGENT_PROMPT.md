# dotnet-rs Follow-up Task Prompt

You are continuing performance work for `dotnet-rs`.

## Target Step
- Step ID: `{{PHASE_ID}}`
- Title: `{{STEP_TITLE}}`

## Checklist Status Summary
{{STATUS_SUMMARY}}

## Dependency Status
{{DEPENDS_ON_STATUS}}

## Relevant Files
{{RELEVANT_FILES}}

## Recent Agent Memory (last 2 entries)
{{AGENT_MEMORY_TAIL}}

## Required Workflow

1. Read `AGENTS.md` and the subsystem docs relevant to this step before editing code.
2. Read all files listed in **Relevant Files** and trace current call/data flow.
3. Implement only the changes required for this step, preserving `'gc` safety and feature-flag correctness.
4. Run at minimum:
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo test -- --nocapture`
   Also run the no-default-features checks if the step touches feature-gated, sync, GC, or pointer code.
5. Run the relevant benchmark workload(s) for this step and compare results against the stored baseline.
6. Append a new session entry to `AGENT_MEMORY.md` using the required template.
7. Update this step status in `CHECKLIST.md` and mark completed atomic tasks.

## Output Requirements

- Report exactly what changed, why, and measured deltas (before/after).
- If measurements regress, include rollback or mitigation options.
- If blocked, document blocker details and the smallest next executable action.
