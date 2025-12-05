#!/usr/bin/env bash
# Remove generated artifacts under soulbrowser-output and other temp roots.
set -euo pipefail

ROOT="$(cd -- "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DRY_RUN=false
if [[ "${1:-}" == "--dry-run" ]]; then
  DRY_RUN=true
fi

TARGETS=(
  "soulbrowser-output"
  "tmp"
  "plan.json"
  "plan_test.json"
)

removed_any=false
for rel in "${TARGETS[@]}"; do
  path="${ROOT}/${rel}"
  if [[ -e "$path" ]]; then
    removed_any=true
    if $DRY_RUN; then
      printf '[dry-run] would remove %s\n' "$path"
    else
      rm -rf -- "$path"
      printf 'Removed %s\n' "$path"
    fi
  fi
done

if ! $removed_any; then
  if $DRY_RUN; then
    echo '[dry-run] nothing to remove'
  else
    echo 'Nothing to remove, workspace already clean.'
  fi
fi
