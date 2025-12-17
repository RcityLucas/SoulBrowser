#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(pwd)
OUTPUT_DIR="$ROOT_DIR/soulbrowser-output"
TENANT="serve-api"
DRY_RUN=0

usage() {
    cat <<'USAGE'
Usage: migrate_execution_outputs.sh [--output-dir DIR] [--tenant ID] [--dry-run]

Moves legacy execution artifacts from soulbrowser-output/tasks/<id>
into the per-tenant layout at soulbrowser-output/tenants/<tenant>/executions/<id>.
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --output-dir)
            shift
            OUTPUT_DIR="$1"
            ;;
        --tenant)
            shift
            TENANT="$1"
            ;;
        --dry-run)
            DRY_RUN=1
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            usage
            exit 1
            ;;
    esac
    shift || break
done

LEGACY_ROOT="${OUTPUT_DIR%/}/tasks"
DEST_ROOT="${OUTPUT_DIR%/}/tenants/$TENANT/executions"

if [[ ! -d "$LEGACY_ROOT" ]]; then
    echo "Legacy directory $LEGACY_ROOT does not exist; nothing to migrate."
    exit 0
fi

mkdir -p "$DEST_ROOT"

shopt -s nullglob
moved=0
skipped=0
for task_dir in "$LEGACY_ROOT"/*; do
    [[ -d "$task_dir" ]] || continue
    task_id="$(basename "$task_dir")"
    target="$DEST_ROOT/$task_id"
    if [[ -e "$target" ]]; then
        echo "Skipping $task_id (already exists in destination)"
        ((skipped++))
        continue
    fi
    if [[ $DRY_RUN -eq 1 ]]; then
        echo "[dry-run] mv '$task_dir' '$target'"
    else
        mv "$task_dir" "$target"
        echo "Moved $task_id -> $target"
    fi
    ((moved++))
done

if [[ $moved -eq 0 ]]; then
    echo "No task directories moved."
else
    echo "Moved $moved task directories into $DEST_ROOT."
fi

if [[ $skipped -gt 0 ]]; then
    echo "Skipped $skipped existing directories."
fi

if [[ $DRY_RUN -eq 0 ]]; then
    rmdir "$LEGACY_ROOT" 2>/dev/null || true
fi
