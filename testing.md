# Reference Audio verification

## Repeatable checks

Run from the project root on macOS:

```sh
./scripts/check.sh
./scripts/build.sh
```

The check script runs rustfmt, Clippy with warnings denied, Rust tests and shell
syntax checks. The build script packages and verifies a locally signed bundle.
External host validation requires separately installed tools.

For a manual MuseScore check, close the host, install the new bundle, reopen a
copy of a test score, select a recording, and verify play/pause, seek, loop,
positive/negative offset, gain and mute. Save and reopen the score to check state
persistence, then close and reopen the editor. Use a disposable score and generated
audio for manual checks. Record the host version, architecture, date and actual results.
