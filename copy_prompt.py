#!/usr/bin/env python3
"""Assemble a follow-up agent prompt for a checklist step and copy it to clipboard."""

from __future__ import annotations

import argparse
import platform
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List


@dataclass
class StepInfo:
    step_id: str
    title: str
    phase_header: str
    status: str
    files: str
    depends_on: str


def normalize_step_id(raw: str) -> str:
    return re.sub(r"[^a-z0-9]", "", raw.lower())


def parse_checklist(path: Path) -> Dict[str, StepInfo]:
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()

    phase_header = ""
    steps: Dict[str, StepInfo] = {}

    i = 0
    while i < len(lines):
        line = lines[i]
        phase_match = re.match(r"^##\s+Phase\s+(.+)$", line)
        if phase_match:
            phase_header = phase_match.group(1).strip()
            i += 1
            continue

        step_match = re.match(r"^###\s+Step\s+([^:]+):\s*(.+)$", line)
        if not step_match:
            i += 1
            continue

        step_id = step_match.group(1).strip()
        title = step_match.group(2).strip()

        status = ""
        files = ""
        depends_on = "none"

        j = i + 1
        while j < len(lines):
            if lines[j].startswith("### Step ") or lines[j].startswith("## Phase "):
                break
            status_match = re.match(r"^-\s+\*\*Status\*\*:\s*(.+)$", lines[j])
            if status_match:
                status = status_match.group(1).strip()
            files_match = re.match(r"^-\s+\*\*Files\*\*:\s*(.+)$", lines[j])
            if files_match:
                files = files_match.group(1).strip()
            depends_match = re.match(r"^-\s+\*\*Depends on\*\*:\s*(.+)$", lines[j])
            if depends_match:
                depends_on = depends_match.group(1).strip()
            j += 1

        key = normalize_step_id(step_id)
        steps[key] = StepInfo(
            step_id=step_id,
            title=title,
            phase_header=phase_header,
            status=status,
            files=files,
            depends_on=depends_on,
        )
        i = j

    return steps


def status_summary(steps: Dict[str, StepInfo]) -> str:
    ordered: List[StepInfo] = list(steps.values())

    def is_completed(status: str) -> bool:
        s = status.lower()
        return "[x]" in s or "completed" in s

    completed = [s for s in ordered if is_completed(s.status)]
    remaining = [s for s in ordered if not is_completed(s.status)]

    remaining_ids = ", ".join(f"{s.step_id}" for s in remaining) if remaining else "none"
    return (
        f"- Completed steps: {len(completed)}/{len(ordered)}\n"
        f"- Remaining steps: {remaining_ids}"
    )


def dependency_status(step: StepInfo, steps: Dict[str, StepInfo]) -> str:
    dep_raw = step.depends_on.strip()
    if dep_raw.lower() == "none":
        return "- none"

    deps = [d.strip() for d in dep_raw.split(",") if d.strip()]
    lines = []
    for dep in deps:
        key = normalize_step_id(dep)
        dep_info = steps.get(key)
        if dep_info is None:
            lines.append(f"- {dep}: NOT FOUND in checklist")
        else:
            lines.append(f"- {dep_info.step_id}: {dep_info.status}")
    return "\n".join(lines)


def memory_tail(path: Path, max_entries: int = 2) -> str:
    text = path.read_text(encoding="utf-8").strip()
    if not text:
        return "(no agent memory entries)"

    # Keep section chunks split by horizontal rule separators used in AGENT_MEMORY.md
    chunks = [c.strip() for c in re.split(r"\n---\s*\n", text) if c.strip()]
    tail = chunks[-max_entries:]
    return "\n\n---\n\n".join(tail)


def copy_to_clipboard(payload: str) -> str:
    system = platform.system().lower()

    try:
        if system == "darwin":
            subprocess.run(
                ["pbcopy"],
                input=payload,
                text=True,
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            return "pbcopy"
        if system == "windows":
            subprocess.run(
                "clip",
                input=payload,
                text=True,
                shell=True,
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            return "clip"

        # Default to Linux/Unix
        subprocess.run(
            ["xclip", "-selection", "clipboard", "-i"],
            input=payload,
            text=True,
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return "xclip"
    except FileNotFoundError:
        return "none"
    except subprocess.CalledProcessError:
        return "none"


def main() -> int:
    parser = argparse.ArgumentParser(description="Assemble and copy a follow-up agent prompt.")
    parser.add_argument("step_id", help="Checklist step ID, for example: 2a")
    args = parser.parse_args()

    root = Path(__file__).resolve().parent
    prompt_template_path = root / "AGENT_PROMPT.md"
    checklist_path = root / "CHECKLIST.md"
    memory_path = root / "AGENT_MEMORY.md"

    for p in (prompt_template_path, checklist_path, memory_path):
        if not p.exists():
            print(f"Missing required file: {p}", file=sys.stderr)
            return 1

    steps = parse_checklist(checklist_path)
    target_key = normalize_step_id(args.step_id)
    target = steps.get(target_key)

    if target is None:
        print(f"Step '{args.step_id}' was not found in CHECKLIST.md", file=sys.stderr)
        known = ", ".join(sorted(s.step_id for s in steps.values()))
        print(f"Known step IDs: {known}", file=sys.stderr)
        return 1

    template = prompt_template_path.read_text(encoding="utf-8")

    rendered = template
    rendered = rendered.replace("{{PHASE_ID}}", target.step_id)
    rendered = rendered.replace("{{STEP_TITLE}}", target.title)
    rendered = rendered.replace("{{STATUS_SUMMARY}}", status_summary(steps))
    rendered = rendered.replace("{{AGENT_MEMORY_TAIL}}", memory_tail(memory_path, max_entries=2))
    rendered = rendered.replace("{{RELEVANT_FILES}}", target.files)
    rendered = rendered.replace("{{DEPENDS_ON_STATUS}}", dependency_status(target, steps))

    tool = copy_to_clipboard(rendered)
    if tool == "none":
        print("Clipboard copy skipped (no supported clipboard tool found).", file=sys.stderr)
    else:
        print(f"Copied prompt to clipboard via {tool}.", file=sys.stderr)

    print(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
