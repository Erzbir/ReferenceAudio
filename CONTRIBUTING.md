# Contributing

## Development environment

Use macOS 12 or later, Xcode Command Line Tools, Python 3 and Rust via rustup.
`rust-toolchain.toml` selects stable Rust with rustfmt and Clippy.
Dependencies are pinned by `Cargo.lock`.
The deployment target defaults to macOS 12 through `.cargo/config.toml`.

From the project root:

```sh
cargo fmt --all
./scripts/check.sh
./scripts/build.sh
```

The build produces a locally signed bundle at `target/bundle/ReferenceAudio.vst3` for
the current Mac's architecture. `CARGO_TARGET_DIR` is supported. Build artifacts
and archived releases are ignored by Git. Commit `Cargo.lock` for this application.

## File naming

- Use lowercase kebab-case for custom documentation, metadata templates, scripts
  and test fixtures, such as `testing.md`, `module-info.json` and
  `legacy-state.bin`. Use `.sh` for shell scripts and keep them under `scripts/`.
- Use snake_case for Rust modules, such as `io_tests.rs`, following Rust conventions.
- Preserve conventional entry points: `README.md`, `LICENSE`,
  `CONTRIBUTING.md`, `Cargo.toml` and `Cargo.lock`.
- Keep tool-defined names such as `.editorconfig`, `rust-toolchain.toml` and
  `rustfmt.toml`. Generated macOS/VST3 bundles retain `Info.plist`, `moduleinfo.json`,
  `ReferenceAudio.vst3` and the `ReferenceAudio` executable for host compatibility.
- Rename references alongside files. Preserve the contents of example recordings,
  scores and historical release artifacts when changing their external filenames.

## Changes and compatibility

Keep changes focused and describe their behavior and verification. Use rustfmt
for Rust code, four spaces for indentation, and LF line endings. The `define_class!`
body in `src/editor.rs` contains Rust items that rustfmt does not automatically
format; keep those items consistent with surrounding code. Use English for code,
UI labels and project documentation.

Preserve the VST class ID, parameter IDs, bundle ID, executable name and saved-state
layout when making compatible changes. The legacy state fixture is a compatibility
contract. Audio callbacks must not allocate, lock or reclaim audio buffers. Decode
and reclaim on background threads, and perform AppKit operations on the main thread.
Document safety assumptions around new unsafe code.

Run `./scripts/check.sh` before submitting changes. For playback, VST lifecycle or
editor changes, also run the manual host checks in `testing.md`. Automated Rust
tests do not establish MuseScore integration or official VST3 validator compliance.

## Releases

1. Update the package version in `Cargo.toml`, refresh `Cargo.lock` with Cargo,
   and document user-visible changes in the release notes.
2. Run the checks and build. Packaging writes Cargo's version into both bundle
   metadata files; do not edit generated metadata under `target/bundle/`.
3. Run the VST3 validator and manual MuseScore checks and record dated results.
4. Distribute the generated `ReferenceAudio.vst3`, README, LICENSE and third-party
   notices at the release root, with `install.sh` and `uninstall.sh` under `scripts/`.
   Keep this directory layout so `./scripts/install.sh` can locate the bundle.

The original prebuilt 2.0.0 bundle is preserved under `target/archive/`. The installer
prefers `target/bundle/ReferenceAudio.vst3`, then `target/archive/ReferenceAudio.vst3`, then a
bundle at the release root in an extracted distribution. Installation overwrites an existing user-level plug-in. Close MuseScore before replacing its plug-in.

When changing dependencies, update `third-party-licenses.txt` along with
`Cargo.lock`. Preserve implementation references in `README.md`.
