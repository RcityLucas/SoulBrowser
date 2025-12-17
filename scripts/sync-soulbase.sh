#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"
TARGET_DIR="$ROOT_DIR/third_party/soul-base"

echo "[sync-soulbase] refreshing third_party/soul-base"
if [ -d "$TARGET_DIR/.git" ]; then
  echo "[sync-soulbase] detected git repository, updating"
  git -C "$TARGET_DIR" fetch --tags origin >/dev/null 2>&1 || true
  git -C "$TARGET_DIR" pull --ff-only >/dev/null 2>&1 || true
  exit 0
fi

if [ -z "${SOULBASE_SOURCE:-}" ]; then
  cat <<MSG
[sync-soulbase] missing SOULBASE_SOURCE; please set it to a local checkout or mirror, e.g.:
  SOULBASE_SOURCE=/path/to/soul-base ./scripts/sync-soulbase.sh
MSG
  exit 1
fi

rsync -a --delete "$SOULBASE_SOURCE/" "$TARGET_DIR/"
echo "[sync-soulbase] sync complete"
