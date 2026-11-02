# Reference Audio

A macOS VST3 plug-in for playing a reference recording alongside a MuseScore Studio 4 score.

Implemented in Rust with a native AppKit editor and macOS AudioToolbox decoding.

## Install

Requires MuseScore Studio 4 on macOS 12.0 or later and a plug-in built for your Mac’s architecture.

For a source checkout, follow [Build](#build) first. For a packaged release,
extract the complete folder with `ReferenceAudio.vst3` at its root and the
installation scripts under `scripts/`.

Save your scores and quit MuseScore Studio 4, then run from the project or release root:

```sh
./scripts/install.sh
```

The installer checks `target/bundle/`, then `target/archive/` if a local archive
exists, then the extracted release root. In a source checkout, Cargo and Python 3
are required to locate the output directory; use the same `CARGO_TARGET_DIR`
setting as the build.

The plug-in is installed to `~/Library/Audio/Plug-Ins/VST3/ReferenceAudio.vst3`.
An existing installation is overwritten. Reopen MuseScore Studio 4 after installation.

To uninstall, quit MuseScore and run `./scripts/uninstall.sh`. This moves the
installed plug-in to the Trash.

## Use

1. Add a dedicated instrument to your score. Its staff may remain empty.
2. In the Mixer, search for **Reference** in that instrument's **Sound** menu, then choose **Reference Audio / ReferenceAudio**.
3. Click the sound slot and choose **Open audio...**, or drop a recording into the editor.
4. Return to the score. MuseScore Studio 4's playback, pause, seek and loop controls drive the recording.

The **Offset** value is the recording time corresponding to score time zero. For example, `12.50` skips the recording's first 12.5 seconds; `-2.00` lets the score play for two seconds before the recording begins. The range is −3600 to +3600 seconds. Press Enter after typing a value, or use the stepper for 0.01-second adjustments. **Reset** returns the offset to zero.

**Level** and **Mute recording** affect the reference recording only. Set the recording track's reverb send to zero for an unprocessed reference signal.

## Save and compatibility

Save the score after selecting a recording or changing settings. The file path, offset, level and mute setting are saved with it. The audio file stays external and is not embedded in `.mscz`; reopen it if you move it.

The plug-in retains the earlier implementation’s VST class ID, parameter IDs, bundle ID and saved-state format. A legacy state fixture is included in the automated tests.

## Build

Requires macOS, stable Rust via rustup, Xcode Command Line Tools and Python 3. The plug-in targets macOS 12.0 or later and builds for the current Mac’s architecture.

```sh
./scripts/check.sh
./scripts/build.sh
```

Output: `target/bundle/ReferenceAudio.vst3`. If `CARGO_TARGET_DIR` is set, the bundle is written to `bundle/ReferenceAudio.vst3` inside that directory. The first build needs network access to download the dependencies pinned by `Cargo.lock`. The C++ VST3 SDK is not required.

The build sets the bundle version from `Cargo.toml` and applies and verifies a local ad-hoc signature. The bundle is not Developer ID signed or notarized.

## Source layout

Paths are relative to the project root. Cargo commands run from this directory.

- `Cargo.toml` and `Cargo.lock`: package metadata and locked dependencies.
- `scripts/`: build, installation, removal and automated checks.
- `resources/`: bundle metadata templates and compatibility fixture.
- `target/`: generated Cargo cache and current bundle under `bundle/` (ignored
  by Git). A local historical bundle may also be present under `archive/`; it is
  not part of the source checkout.
- `src/engine.rs`: background loading, immutable audio publication and host-time playback.
- `src/decode.rs`: AudioToolbox decoding through Rust C FFI.
- `src/state.rs`: the compatible saved-state format.
- `src/vst.rs`: VST3 factory, component, controller, audio callback and parameter handling.
- `src/editor.rs`: the AppKit editor implemented with `objc2`.
- `src/io_tests.rs`: decoding, file replacement and missing-file tests.

See `CONTRIBUTING.md` for development and release conventions, and `testing.md`
for automated and manual verification.

## Scope

The plug-in opens mono and stereo recordings through macOS AudioToolbox; format support depends on the system decoder. Multichannel recordings must first be converted to mono or stereo. Each recording’s decoded 32-bit floating-point sample data is limited to 512 MiB; this is not the compressed file size. The recording plays at its original speed, with silence outside its bounds. There is no beat detection, transcription or time stretching.

The audio callback takes no mutex and does not allocate or reclaim audio buffers. File decoding and buffer reclamation run outside the audio callback. Native UI operations run on the main thread. The library is pinned after first editor attachment because Objective-C classes remain registered for the process lifetime.

## License and sources

Project code is MIT licensed. `Cargo.lock` pins Rust dependencies; their notices are in `third-party-licenses.txt`. This project is not affiliated with Muse Group or Steinberg.

Implementation references include [vst3-rs](https://github.com/coupler-rs/vst3-rs)
interface and factory examples, [objc2](https://github.com/madsmtm/objc2) AppKit
examples, and Apple AudioToolbox headers. The legacy state fixture comes from
an earlier release test score using generated audio.
