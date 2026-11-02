#!/bin/zsh
set -euo pipefail
project_dir="${0:A:h:h}"
cd "$project_dir"
if [[ "$(uname -s)" != Darwin ]]; then
  print -u2 'Reference Audio requires macOS and Xcode Command Line Tools.'
  exit 1
fi
for tool in cargo rustc python3 install_name_tool codesign; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    print -u2 "Required tool not found: $tool"
    exit 1
  fi
done
# Build for this Mac explicitly, even if a user has configured a Cargo target.
host_target="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --release --locked --target "$host_target"
metadata_file="$(mktemp -t reference-audio-metadata)"
trap 'rm -f "$metadata_file"' EXIT
cargo metadata --locked --no-deps --format-version 1 > "$metadata_file"
bundle="$(python3 - "$metadata_file" <<'PYDIR'
import json
import sys
from pathlib import Path
metadata = json.loads(Path(sys.argv[1]).read_text())
print(Path(metadata['target_directory']) / 'bundle/ReferenceAudio.vst3')
PYDIR
)"
python3 - "$metadata_file" "$project_dir" "$bundle" "$host_target" <<'PY'
import json
import plistlib
import shutil
import sys
from pathlib import Path

metadata_path, root, bundle, host = sys.argv[1:]
root, bundle = Path(root), Path(bundle)
metadata = json.loads(Path(metadata_path).read_text())
package = next(p for p in metadata['packages'] if p['name'] == 'reference-audio')
version = package['version']
library = Path(metadata['target_directory']) / host / 'release/libreference_audio.dylib'
(bundle / 'Contents/MacOS').mkdir(parents=True, exist_ok=True)
resources = bundle / 'Contents/Resources'
resources.mkdir(parents=True, exist_ok=True)
shutil.copy2(library, bundle / 'Contents/MacOS/ReferenceAudio')
with (root / 'resources/info.plist').open('rb') as f:
    info = plistlib.load(f)
info['CFBundleVersion'] = info['CFBundleShortVersionString'] = version
with (bundle / 'Contents/Info.plist').open('wb') as f:
    plistlib.dump(info, f, sort_keys=False)
module = json.loads((root / 'resources/module-info.json').read_text())
module['Version'] = version
for cls in module['Classes']:
    cls['Version'] = version
(resources / 'moduleinfo.json').write_text(json.dumps(module, indent=2) + '\n')
for name in ('LICENSE', 'third-party-licenses.txt'):
    shutil.copy2(root / name, resources / name)
PY
install_name_tool -id '@rpath/ReferenceAudio' "$bundle/Contents/MacOS/ReferenceAudio"
codesign --force --deep --sign - "$bundle"
codesign --verify --deep --strict "$bundle"
print "Built: $bundle"
