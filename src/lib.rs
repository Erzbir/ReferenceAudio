#![allow(non_snake_case)]

// The VST3 component, audio engine and native editor are implemented in Rust.
mod decode;
mod editor;
mod engine;
#[cfg(test)]
mod io_tests;
mod state;
mod vst;
