#!/usr/bin/env python3
"""
copy_prompt.py — Generate a filled-in agent prompt for a given checklist step.

Usage:
    python copy_prompt.py 2.1          # copies to clipboard
    python copy_prompt.py 2.1 --stdout # prints to stdout
"""

import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).parent
CHECKLIST = REPO / "CHECKLIST.md"
REVIEW = REPO / "REVIEW.md"
AGENT_PROMPT = REPO / "AGENT_PROMPT.md"
AGENT_MEMORY = REPO / "AGENT_MEMORY.md"


def die(msg: str) -> None:
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(1)


def copy_to_clipboard(text: str) -> None:
    """Try wl-copy, then xclip, then xsel. Error if none available."""
    for cmd in (
        ["wl-copy"],
        ["xclip", "-selection", "clipboard"],
        ["xsel", "--clipboard", "--input"],
    ):
        try:
            proc = subprocess.run(cmd, input=text, text=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if proc.returncode == 0:
                return
        except FileNotFoundError:
            continue
    die(
        "No clipboard tool found. Install wl-copy (Wayland), xclip, or xsel. "
        "Use --stdout to print instead."
    )


def parse_checklist(text: str) -> dict[str, tuple[str, str]]:
    """
    Parse CHECKLIST.md and return {step_id: (title, phase_name)}.
    Handles both checked and unchecked items.
    """
    steps: dict[str, tuple[str, str]] = {}
    current_phase = ""
    for line in text.splitlines():
        # Phase header: ## Phase N: Name
        phase_match = re.match(r"^##\s+Phase\s+\d+[:\s]+(.+)", line)
        if phase_match:
            current_phase = phase_match.group(1).strip()
            continue
        # Step line: - [ ] N.M Description — refs ...
        step_match = re.match(r"^-\s+\[[ xX]\]\s+(\d+\.\d+)\s+(.*)", line)
        if step_match:
            step_id = step_match.group(1)
            rest = step_match.group(2)
            # Strip the " — refs ..." tail if present
            title = re.split(r"\s+—\s+refs\s+", rest)[0].strip()
            steps[step_id] = (title, current_phase)
    return steps


def extract_review_section(review_text: str, step_id: str, step_title: str) -> str:
    """
    Try to extract the REVIEW.md section most relevant to this step.

    Heuristic: look for any anchor or heading that mentions refs cited in CHECKLIST
    for this step. Falls back to the full executive summary if nothing specific found.
    """
    # Find the CHECKLIST line for this step to extract its ref anchor
    checklist_text = CHECKLIST.read_text(encoding="utf-8")
    ref_anchor = None
    for line in checklist_text.splitlines():
        if re.match(rf"^-\s+\[[ xX]\]\s+{re.escape(step_id)}\s+", line):
            ref_match = re.search(r"refs REVIEW\.md#(\S+)", line)
            if ref_match:
                ref_anchor = ref_match.group(1)
            break

    if not ref_anchor:
        # No specific anchor; return exec summary
        return _extract_exec_summary(review_text)

    # Find the heading that corresponds to this anchor. REVIEW.md uses:
    #   <a id="F-DEAD-001"></a>
    #   #### F-DEAD-001 — Title
    # or a plain heading like:
    #   ### Phase 1: ...
    anchor_pattern = re.compile(
        rf'<a\s+id="{re.escape(ref_anchor)}"></a>|^#+\s+{re.escape(ref_anchor)}',
        re.MULTILINE,
    )
    match = anchor_pattern.search(review_text)
    if not match:
        return _extract_exec_summary(review_text)

    # The anchor may be a bare <a id="..."></a> tag (on its own line before the heading).
    # Advance past it to find the actual heading line.
    pos = match.end()
    # Skip past the anchor line to find the next heading
    heading_search_start = pos
    next_heading_from_anchor = re.compile(r"^(#+)\s", re.MULTILINE)
    heading_match = next_heading_from_anchor.search(review_text, heading_search_start)
    if heading_match:
        start = heading_match.start()
        level = len(heading_match.group(1))
    else:
        start = match.start()
        level = 4  # default to h4

    # Find end: next heading at same or higher level (fewer or equal # characters)
    next_heading = re.compile(rf"^#{{1,{level}}}\s", re.MULTILINE)
    end_match = next_heading.search(review_text, start + 1)
    end = end_match.start() if end_match else len(review_text)

    section = review_text[start:end].strip()
    # Limit to 3000 chars to avoid overwhelming context
    if len(section) > 3000:
        section = section[:3000] + "\n\n[... section truncated, see REVIEW.md for full text]"
    return section


def _extract_exec_summary(review_text: str) -> str:
    """Extract the Executive Summary section from REVIEW.md."""
    match = re.search(r"^## Executive Summary\b", review_text, re.MULTILINE)
    if not match:
        return "(Could not extract executive summary — see REVIEW.md)"
    start = match.start()
    end_match = re.search(r"^## \w", review_text[start + 1:], re.MULTILINE)
    end = start + 1 + end_match.start() if end_match else start + 2000
    return review_text[start:end].strip()[:2000]


def main() -> None:
    args = sys.argv[1:]
    stdout_mode = "--stdout" in args
    args = [a for a in args if a != "--stdout"]

    if not args:
        die("Usage: python copy_prompt.py <STEP_ID> [--stdout]\nExample: python copy_prompt.py 2.1")

    step_id = args[0].strip()

    # Validate files exist
    for p in (CHECKLIST, REVIEW, AGENT_PROMPT, AGENT_MEMORY):
        if not p.exists():
            die(f"{p.name} not found at {p}. Run from the repo root.")

    checklist_text = CHECKLIST.read_text(encoding="utf-8")
    review_text = REVIEW.read_text(encoding="utf-8")
    template = AGENT_PROMPT.read_text(encoding="utf-8")
    memory_text = AGENT_MEMORY.read_text(encoding="utf-8")

    steps = parse_checklist(checklist_text)
    if step_id not in steps:
        available = ", ".join(sorted(steps))
        die(f"Step '{step_id}' not found in CHECKLIST.md.\nAvailable steps: {available}")

    step_title, phase_name = steps[step_id]

    # Build step description from the checklist line itself (strip refs suffix)
    step_desc_line = ""
    for line in checklist_text.splitlines():
        if re.match(rf"^-\s+\[[ xX]\]\s+{re.escape(step_id)}\s+", line):
            step_desc_line = re.split(r"\s+—\s+refs\s+", line)[0]
            step_desc_line = re.sub(r"^-\s+\[[ xX]\]\s+\S+\s+", "", step_desc_line).strip()
            break

    review_section = extract_review_section(review_text, step_id, step_title)

    # Trim memory to last 4000 chars so it doesn't dominate the prompt
    if len(memory_text) > 4000:
        memory_text = "[... earlier entries omitted ...]\n\n" + memory_text[-4000:]

    filled = (
        template
        .replace("{{STEP_ID}}", step_id)
        .replace("{{STEP_TITLE}}", step_title)
        .replace("{{STEP_DESCRIPTION}}", step_desc_line)
        .replace("{{REVIEW_SECTION}}", review_section)
        .replace("{{PRIOR_MEMORY}}", memory_text)
        .replace("{{CHECKLIST_SNAPSHOT}}", checklist_text)
    )

    if stdout_mode:
        print(filled)
    else:
        copy_to_clipboard(filled)
        print(f"Copied prompt for step {step_id} ({step_title!r}) to clipboard.")
        print(f"Prompt length: {len(filled):,} characters.")


if __name__ == "__main__":
    main()
