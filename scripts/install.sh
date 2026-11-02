#!/bin/zsh
set -euo pipefail
package_dir="${0:A:h:h}"
# Prefer a fresh local build; also support archived and extracted releases.
if [[ -f "$package_dir/Cargo.toml" ]]; then
  cd "$package_dir"
  artifact_dir="$(cargo metadata --locked --offline --no-deps --format-version 1 | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
else
  artifact_dir="$package_dir/target"
fi
source_bundle="$artifact_dir/bundle/ReferenceAudio.vst3"
if [[ ! -d "$source_bundle" ]]; then
  source_bundle="$artifact_dir/archive/ReferenceAudio.vst3"
fi
if [[ ! -d "$source_bundle" ]]; then
  source_bundle="$package_dir/ReferenceAudio.vst3"
fi
target_dir="$HOME/Library/Audio/Plug-Ins/VST3"
target_bundle="$target_dir/ReferenceAudio.vst3"
if [[ ! -d "$source_bundle" ]]; then
  print -u2 'ReferenceAudio.vst3 was not found. Run ./scripts/build.sh or extract a complete release folder first.'
  exit 1
fi
codesign --verify --deep --strict "$source_bundle"
mkdir -p "$target_dir"
ditto "$source_bundle" "$target_bundle"
print 'Reference Audio is installed. Save your scores, quit MuseScore and reopen it.'
print 'In the Mixer, search for Reference in the Sound menu of a dedicated empty track.'
