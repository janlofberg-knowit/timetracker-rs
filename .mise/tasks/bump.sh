#!/usr/bin/env bash
set -euo pipefail

level="${1:-}"

if [[ ! "$level" =~ ^(major|minor|patch)$ ]]; then
  echo "Usage: mise bump <major|minor|patch>"
  exit 1
fi

cargo set-version --bump "$level"

version=$(cargo metadata --no-deps --format-version 1 \
  | jq -r '.packages[0].version')

git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to $version"
