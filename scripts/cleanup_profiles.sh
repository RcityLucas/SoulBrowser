#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")"/.. && pwd)"
cd "$ROOT_DIR"

profiles=(.soulbrowser-profile-*)
count=0
for dir in "${profiles[@]}"; do
  if [[ -d $dir ]];
  then
    rm -rf "$dir"
    ((count++))
  fi
done

echo "Removed $count temporary Chrome profile directories." 
