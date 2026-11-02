use crate::{decode, engine::Engine};
use std::{
    fs,
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "reference-audio-rust-{}-{}.wav",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut f = fs::File::create_new(&path).unwrap();
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend((36u32 + 2000).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1000u32.to_le_bytes());
        bytes.extend(2000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(2000u32.to_le_bytes());
        for i in 0..1000 {
            bytes.extend((i as i16 * 20).to_le_bytes());
        }
        f.write_all(&bytes).unwrap();
        Self(path)
    }
    fn path(&self) -> String {
        self.0.to_str().unwrap().to_owned()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn wait(e: &Engine) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while e.metadata().loading && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!e.metadata().loading, "loader timed out");
}
#[test]
fn decode_wav_and_cancel() {
    let f = Fixture::new();
    let a = decode::decode(&f.path(), || false).unwrap();
    assert_eq!(a.channels, 1);
    assert_eq!(a.frames(), 1000);
    assert_eq!(a.rate, 1000.);
    assert!((a.samples[100] - 2000. / 32768.).abs() < 1e-6);
    assert!(decode::decode(&f.path(), || true).is_err());
}
#[test]
fn replacement_missing_file_and_mono_playback() {
    let f = Fixture::new();
    let e = Engine::new();
    e.shared.gain.set(1.);
    e.load(f.path());
    wait(&e);
    assert!(e.metadata().ready);
    let (mut l, mut r) = ([0.; 64], [0.; 64]);
    e.render(&mut l, &mut r, 1000., 100, true);
    assert!((l[0] - 2000. / 32768.).abs() < 1e-6);
    assert_eq!(l, r);
    e.load(format!("{}.missing", f.path()));
    wait(&e);
    e.render(&mut l, &mut r, 1000., 100, true);
    assert!(!e.metadata().ready);
    assert!(l.iter().all(|v| *v == 0.));
    for _ in 0..100 {
        e.load(format!("{}.missing", f.path()));
        e.load(f.path());
    }
    wait(&e);
    assert!(e.metadata().ready);
    assert_eq!(e.metadata().path, f.path());
}
