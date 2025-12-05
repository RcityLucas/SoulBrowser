#!/usr/bin/env python3
"""Validate that deprecated docs/scripts are not reintroduced and .example files stay curated."""
from __future__ import annotations

import os
import subprocess
import sys
from typing import Dict, Iterable, List

DEPRECATED_BASENAMES = {
    "README_BROWSER_FIX.txt": "migrated into docs/guides/TROUBLESHOOTING.md",
    "WINDOWS_ACCESS_FIX.md": "merged into docs/guides/TROUBLESHOOTING.md",
    "FRONTEND_PORT_FIX.md": "merged into docs/guides/TROUBLESHOOTING.md",
}

RELOCATED_FILES: Dict[str, str] = {
    "API_TEST_RESULTS.md": "docs/history/API_TEST_RESULTS.md",
    "BACKEND_TEST_REPORT.md": "docs/history/BACKEND_TEST_REPORT.md",
    "PHASE_0.5_PROGRESS.md": "docs/history/PHASE_0.5_PROGRESS.md",
    "PLAN_EVALUATION_AND_RECOMMENDATIONS.md": "docs/history/PLAN_EVALUATION_AND_RECOMMENDATIONS.md",
}

ALLOWED_EXAMPLE_FILES = {
    "config/config.yaml.example",
}

PERCEIVE_PREFIX = "Perceive_API_"


def _capture_repo_files() -> List[str]:
    try:
        proc = subprocess.run(
            ["rg", "--files"],
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError) as err:
        print(f"check_dead_files: unable to enumerate files via rg: {err}", file=sys.stderr)
        return []
    return [line.strip() for line in proc.stdout.splitlines() if line.strip()]


def _iter_files_from_stdin() -> Iterable[str]:
    for raw in sys.stdin:
        raw = raw.strip().lstrip("./")
        if raw:
            yield raw


def gather_files() -> List[str]:
    if sys.stdin.isatty():
        files = _capture_repo_files()
    else:
        files = list(_iter_files_from_stdin())
        if not files:
            files = _capture_repo_files()
    return files


def main() -> int:
    files = gather_files()
    if not files:
        print("check_dead_files: no files to scan", file=sys.stderr)
        return 0

    failures: List[str] = []

    for path in files:
        normalized = path.lstrip("./")
        base = os.path.basename(normalized)

        if base in DEPRECATED_BASENAMES:
            failures.append(f"{normalized} :: {DEPRECATED_BASENAMES[base]}")

        if base.startswith(PERCEIVE_PREFIX):
            failures.append(
                f"{normalized} :: Perceive_API reports were consolidated into docs/guides/TROUBLESHOOTING.md"
            )

        if normalized.endswith(".example") and normalized not in ALLOWED_EXAMPLE_FILES:
            failures.append(
                f"{normalized} :: unexpected .example file (only {', '.join(sorted(ALLOWED_EXAMPLE_FILES))} allowed)"
            )

        allowed_location = RELOCATED_FILES.get(base)
        if allowed_location and normalized != allowed_location:
            failures.append(f"{normalized} :: '{base}' must reside at {allowed_location}")

    if failures:
        print(f"Dead-file check found {len(failures)} issue(s):", file=sys.stderr)
        for entry in failures:
            print(f" - {entry}", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
