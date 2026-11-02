#!/bin/zsh
set -euo pipefail
target_bundle="$HOME/Library/Audio/Plug-Ins/VST3/ReferenceAudio.vst3"
if [[ -d "$target_bundle" ]]; then
  mkdir -p "$HOME/.Trash"
  mv "$target_bundle" "$HOME/.Trash/ReferenceAudio-$(date +%Y%m%d-%H%M%S).vst3"
  print 'Reference Audio was moved to the Trash. Quit MuseScore and reopen it.'
else
  print 'Reference Audio was not found in the current user VST3 directory.'
fi
