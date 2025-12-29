#!/usr/bin/env python3
"""Validate agent plans reference only supported custom tool identifiers."""
from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Iterable, List

ROOT = Path(__file__).resolve().parents[2]
ALLOWLIST_PATH = ROOT / "config" / "planner" / "custom_tool_allowlist.json"
PLAN_DIR = ROOT / "docs" / "plans"
DEFAULT_PATTERNS = ("plan*.json",)

def load_allowlist() -> dict:
    try:
        data = json.loads(ALLOWLIST_PATH.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:  # pragma: no cover - must exist in repo
        raise SystemExit(f"allowlist file missing: {ALLOWLIST_PATH}") from exc
    except json.JSONDecodeError as exc:  # pragma: no cover - must be valid JSON
        raise SystemExit(f"allowlist file is invalid JSON: {exc}") from exc
    for key in ("names", "aliases", "prefixes", "suffixes"):
        data.setdefault(key, [])
    # normalise for case-insensitive comparisons
    data["names"] = {entry.strip().lower() for entry in data["names"]}
    data["aliases"] = {entry.strip().lower() for entry in data["aliases"]}
    data["prefixes"] = tuple(entry.strip().lower() for entry in data["prefixes"])
    data["suffixes"] = tuple(entry.strip().lower() for entry in data["suffixes"])
    return data

def discover_plan_files(cli_args: List[str]) -> List[Path]:
    if cli_args:
        files: List[Path] = []
        for arg in cli_args:
            candidate = Path(arg)
            path = (ROOT / candidate).resolve() if not candidate.is_absolute() else candidate.resolve()
            if not path.exists():
                raise SystemExit(f"plan file not found: {arg}")
            files.append(path)
        return files

    plan_files = {path.resolve() for pattern in DEFAULT_PATTERNS for path in ROOT.glob(pattern) if path.is_file()}
    if PLAN_DIR.exists():
        plan_files.update(
            path.resolve()
            for pattern in DEFAULT_PATTERNS
            for path in PLAN_DIR.rglob(pattern)
            if path.is_file()
        )
    return sorted(plan_files)

def iter_custom_tool_names(plan: dict) -> Iterable[tuple[str, str]]:
    steps = plan.get("steps", [])
    for idx, step in enumerate(steps, start=1):
        tool = step.get("tool", {})
        kind = tool.get("kind", {})
        custom = kind.get("Custom")
        if not isinstance(custom, dict):
            continue
        name = custom.get("name") or ""
        title = step.get("title") or f"step-{idx}"
        yield name, title

def is_allowed(name: str, allowlist: dict) -> bool:
    ident = name.strip().lower()
    if not ident:
        return True  # empty names handled elsewhere
    if ident in allowlist["names"] or ident in allowlist["aliases"]:
        return True
    if allowlist["prefixes"] and any(ident.startswith(prefix) for prefix in allowlist["prefixes"]):
        return True
    if allowlist["suffixes"] and any(ident.endswith(suffix) for suffix in allowlist["suffixes"]):
        return True
    return False

def lint_plan(path: Path, allowlist: dict) -> List[str]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return [f"{path}: invalid JSON - {exc}"]
    issues = []
    for name, title in iter_custom_tool_names(data):
        if not name:
            issues.append(f"{path}: step '{title}' is missing a custom tool name")
            continue
        if not is_allowed(name, allowlist):
            issues.append(
                f"{path}: step '{title}' uses unsupported custom tool '{name}'. "
                f"See {ALLOWLIST_PATH} for allowed identifiers."
            )
    return issues

def main(argv: List[str]) -> int:
    allowlist = load_allowlist()
    plan_files = discover_plan_files(argv[1:])
    if not plan_files:
        print("lint_plan_tools: no plan*.json files found")
        return 0
    all_issues: List[str] = []
    for path in plan_files:
        all_issues.extend(lint_plan(path, allowlist))
    if all_issues:
        print("\n".join(all_issues))
        return 1
    print(f"lint_plan_tools: {len(plan_files)} plan file(s) verified")
    return 0

if __name__ == "__main__":
    sys.exit(main(sys.argv))
