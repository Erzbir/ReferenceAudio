#!/bin/zsh
set -euo pipefail
project_dir="${0:A:h:h}"
cd "$project_dir"
if [[ "$(uname -s)" != Darwin ]]; then
  print -u2 'Reference Audio checks require macOS (AppKit and AudioToolbox).'
  exit 1
fi
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
for script in scripts/*.sh; do
  zsh -n "$script"
done
print 'All automated checks passed.'
