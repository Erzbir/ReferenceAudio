use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

pub struct AtomicFloat(AtomicU64);
impl AtomicFloat {
    pub fn new(value: f64) -> Self {
        Self(AtomicU64::new(value.to_bits()))
    }
    pub fn get(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::Relaxed))
    }
    pub fn set(&self, value: f64) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }
}

pub struct Audio {
    pub rate: f64,
    pub channels: usize,
    pub samples: Vec<f32>,
}
impl Audio {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels
    }
}

// Publication and reclamation run on the loader/control thread. The audio reader
// uses a hazard pointer, performs no allocation, and never takes a mutex.
struct AudioStore {
    current: AtomicPtr<Audio>,
    hazard: AtomicPtr<Audio>,
    reader_active: AtomicBool,
    #[allow(clippy::vec_box)] // Each allocation must keep its address during publication.
    owned: Mutex<Vec<Box<Audio>>>,
}
impl AudioStore {
    fn new() -> Self {
        Self {
            current: AtomicPtr::new(ptr::null_mut()),
            hazard: AtomicPtr::new(ptr::null_mut()),
            reader_active: AtomicBool::new(false),
            owned: Mutex::new(Vec::new()),
        }
    }
    fn publish(&self, audio: Option<Audio>) {
        let mut owned = self.owned.lock().unwrap_or_else(|p| p.into_inner());
        let next = if let Some(audio) = audio {
            let mut audio = Box::new(audio);
            let address = &mut *audio as *mut Audio;
            owned.push(audio);
            address
        } else {
            ptr::null_mut()
        };
        self.current.store(next, Ordering::SeqCst);
        let hazard = self.hazard.load(Ordering::SeqCst);
        owned.retain(|a| ptr::eq(&**a, next) || ptr::eq(&**a, hazard));
    }
    fn read(&self) -> Option<AudioRead<'_>> {
        if self
            .reader_active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return None;
        }
        loop {
            let audio = self.current.load(Ordering::SeqCst);
            self.hazard.store(audio, Ordering::SeqCst);
            if audio == self.current.load(Ordering::SeqCst) {
                return Some(AudioRead { store: self, audio });
            }
        }
    }
}
struct AudioRead<'a> {
    store: &'a AudioStore,
    audio: *const Audio,
}
impl AudioRead<'_> {
    fn get(&self) -> Option<&Audio> {
        // SAFETY: This is the sole reader. After pointer publication and a
        // successful current-pointer recheck, publishers retain this allocation
        // until the guard clears the hazard pointer. No reference escapes it.
        unsafe { self.audio.as_ref() }
    }
}
impl Drop for AudioRead<'_> {
    fn drop(&mut self) {
        self.store.hazard.store(ptr::null_mut(), Ordering::SeqCst);
        self.store.reader_active.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone)]
pub struct Metadata {
    pub path: String,
    pub message: String,
    pub loading: bool,
    pub ready: bool,
    pub duration: f64,
    generation: u64,
}
impl Default for Metadata {
    fn default() -> Self {
        Self {
            path: String::new(),
            message: "Open a reference recording".into(),
            loading: false,
            ready: false,
            duration: 0.,
            generation: 0,
        }
    }
}
pub struct Shared {
    metadata: Mutex<Metadata>,
    changed: Condvar,
    stopping: AtomicBool,
    audio: AudioStore,
    pub gain: AtomicFloat,
    pub offset: AtomicFloat,
    pub muted: AtomicBool,
    pub score_time: AtomicFloat,
    pub playing: AtomicBool,
    last_callback_ms: AtomicU64,
    epoch: Instant,
}
pub struct Engine {
    pub shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl Engine {
    pub fn new() -> Arc<Self> {
        let shared = Arc::new(Shared {
            metadata: Mutex::new(Metadata::default()),
            changed: Condvar::new(),
            stopping: AtomicBool::new(false),
            audio: AudioStore::new(),
            gain: AtomicFloat::new(0.8),
            offset: AtomicFloat::new(0.),
            muted: AtomicBool::new(false),
            score_time: AtomicFloat::new(0.),
            playing: AtomicBool::new(false),
            last_callback_ms: AtomicU64::new(u64::MAX),
            epoch: Instant::now(),
        });
        let worker_shared = shared.clone();
        let worker = thread::Builder::new()
            .name("reference-audio-loader".into())
            .spawn(move || loader(worker_shared))
            .expect("audio loader thread");
        Arc::new(Self {
            shared,
            worker: Mutex::new(Some(worker)),
        })
    }
    pub fn metadata(&self) -> Metadata {
        self.shared
            .metadata
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    pub fn load(&self, path: String) {
        let s = &self.shared;
        let mut m = s.metadata.lock().unwrap_or_else(|p| p.into_inner());
        let generation = m.generation.wrapping_add(1);
        *m = Metadata {
            loading: !path.is_empty(),
            message: if path.is_empty() {
                "Open a reference recording"
            } else {
                "Loading audio..."
            }
            .into(),
            path,
            ready: false,
            duration: 0.,
            generation,
        };
        s.audio.publish(None);
        s.changed.notify_one();
    }
    pub fn recently_playing(&self) -> bool {
        let s = &self.shared;
        let last = s.last_callback_ms.load(Ordering::Relaxed);
        last != u64::MAX
            && (s.epoch.elapsed().as_millis() as u64).saturating_sub(last) < 500
            && s.playing.load(Ordering::Relaxed)
    }
    pub fn render(
        &self,
        left: &mut [f32],
        right: &mut [f32],
        host_rate: f64,
        position: i64,
        playing: bool,
    ) {
        left.fill(0.);
        right.fill(0.);
        let s = &self.shared;
        let valid_rate = host_rate.is_finite() && host_rate > 0.;
        let host_time = if valid_rate {
            position as f64 / host_rate
        } else {
            0.
        };
        s.score_time.set(host_time);
        s.playing.store(playing, Ordering::Relaxed);
        s.last_callback_ms
            .store(s.epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
        if !playing || !valid_rate || s.muted.load(Ordering::Relaxed) {
            return;
        }
        let Some(read) = s.audio.read() else {
            return;
        };
        let Some(audio) = read.get() else {
            return;
        };
        if audio.channels == 0 || audio.frames() == 0 {
            return;
        }
        let level = s.gain.get().clamp(0., 1.) as f32;
        let start = (host_time + s.offset.get()) * audio.rate;
        let step = audio.rate / host_rate;
        let frames = audio.frames();
        for (i, (left, right)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            let at = start + i as f64 * step;
            if !at.is_finite() || at < 0. || at >= frames as f64 {
                continue;
            }
            let frame = at as usize;
            let next = (frame + 1).min(frames - 1);
            let fraction = (at - frame as f64) as f32;
            let sample = |channel: usize| {
                let channel = if audio.channels == 1 { 0 } else { channel };
                let a = audio.samples[frame * audio.channels + channel];
                let b = audio.samples[next * audio.channels + channel];
                (a + (b - a) * fraction) * level
            };
            *left = sample(0);
            *right = sample(1);
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        // Hold the same mutex as the condvar predicate to avoid a lost wakeup.
        {
            let _m = self
                .shared
                .metadata
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            self.shared.stopping.store(true, Ordering::SeqCst);
            self.shared.changed.notify_one();
        }
        if let Some(worker) = self
            .worker
            .get_mut()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            let _ = worker.join();
        }
    }
}
fn loader(s: Arc<Shared>) {
    let mut completed = 0;
    loop {
        let request = {
            let mut m = s.metadata.lock().unwrap_or_else(|p| p.into_inner());
            while !s.stopping.load(Ordering::SeqCst) && m.generation == completed {
                m = s.changed.wait(m).unwrap_or_else(|p| p.into_inner());
            }
            if s.stopping.load(Ordering::SeqCst) {
                return;
            }
            m.clone()
        };
        let cancelled = || {
            s.stopping.load(Ordering::SeqCst)
                || s.metadata
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .generation
                    != request.generation
        };
        let result = if request.path.is_empty() {
            Err("Open a reference recording".into())
        } else {
            crate::decode::decode(&request.path, cancelled)
        };
        let mut m = s.metadata.lock().unwrap_or_else(|p| p.into_inner());
        completed = request.generation;
        if s.stopping.load(Ordering::SeqCst) {
            return;
        }
        if m.generation != request.generation {
            continue;
        }
        m.loading = false;
        match result {
            Ok(audio) => {
                m.duration = audio.frames() as f64 / audio.rate;
                m.ready = true;
                m.message = "Loaded. Start playback in MuseScore.".into();
                s.audio.publish(Some(audio));
            }
            Err(error) => {
                m.ready = false;
                m.message = error;
                s.audio.publish(None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Audio {
        Audio {
            rate: 1000.,
            channels: 2,
            samples: (0..1000)
                .flat_map(|i| [i as f32 / 1000., -i as f32 / 1000.])
                .collect(),
        }
    }
    fn engine() -> Arc<Engine> {
        let e = Engine::new();
        e.shared.gain.set(1.);
        e.shared.audio.publish(Some(fixture()));
        e
    }
    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-5, "{a} != {b}");
    }
    #[test]
    fn transport() {
        let e = engine();
        let (mut l, mut r) = ([0.; 64], [0.; 64]);
        e.render(&mut l, &mut r, 1000., 400, true);
        close(l[0], 0.4);
        close(r[0], -0.4);
        e.render(&mut l, &mut r, 1000., 400, false);
        assert!(l.iter().chain(&r).all(|x| *x == 0.));
        e.render(&mut l, &mut r, 1000., 100, true);
        close(l[0], 0.1);
    }
    #[test]
    fn offsets_and_bounds() {
        let e = engine();
        let (mut l, mut r) = ([0.; 64], [0.; 64]);
        e.shared.offset.set(0.2);
        e.render(&mut l, &mut r, 1000., 100, true);
        close(l[0], 0.3);
        e.shared.offset.set(-0.1);
        e.render(&mut l, &mut r, 1000., 80, true);
        close(l[0], 0.);
        close(l[30], 0.01);
        e.shared.offset.set(0.);
        e.render(&mut l, &mut r, 1000., 990, true);
        close(l[9], 0.999);
        assert!(l[10..].iter().all(|v| *v == 0.));
    }
    #[test]
    fn resampling_gain_mute_isolation() {
        let e = engine();
        let (mut l, mut r) = ([0.; 64], [0.; 64]);
        e.render(&mut l, &mut r, 2000., 400, true);
        close(l[1], 0.2005);
        e.shared.gain.set(0.25);
        e.render(&mut l, &mut r, 1000., 400, true);
        close(l[0], 0.1);
        e.shared.muted.store(true, Ordering::Relaxed);
        e.render(&mut l, &mut r, 1000., 400, true);
        close(l[0], 0.);
        let other = Engine::new();
        other.render(&mut l, &mut r, 1000., 400, true);
        close(l[0], 0.);
    }
    #[test]
    fn concurrent_replacement_and_shutdown() {
        let e = engine();
        let active = Arc::new(AtomicBool::new(true));
        let reader = e.clone();
        let run = active.clone();
        let t = thread::spawn(move || {
            let (mut l, mut r) = ([0.; 64], [0.; 64]);
            while run.load(Ordering::Relaxed) {
                reader.render(&mut l, &mut r, 1000., 100, true);
            }
        });
        for i in 0..2000 {
            e.shared
                .audio
                .publish(if i % 2 == 0 { Some(fixture()) } else { None });
        }
        active.store(false, Ordering::Relaxed);
        t.join().unwrap();
        for _ in 0..100 {
            drop(Engine::new());
        }
    }
}
