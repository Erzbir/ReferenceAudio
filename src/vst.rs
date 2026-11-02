#![allow(unsafe_op_in_unsafe_fn)]
// VST3 callbacks are an unsafe ABI boundary. The host owns argument buffers;
// callbacks validate null pointers, counts and enum ranges before using them.
use crate::{
    editor,
    engine::{AtomicFloat, Engine},
    state,
};
use std::cell::RefCell;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::{
    atomic::{AtomicBool, AtomicPtr, Ordering},
    Arc,
};
use vst3::{uid, Class, ComPtr, ComRef, ComWrapper, Steinberg::Vst::*, Steinberg::*};

pub const CID: TUID = uid(0x798BDE43, 0x42FC4E10, 0xA2809944, 0x0FA782D1);
pub const GAIN: u32 = 1;
pub const OFFSET: u32 = 2;
pub const MUTE: u32 = 3;
pub const FILE_REVISION: u32 = 4;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub fn cstring(text: &str, dst: &mut [c_char]) {
    dst.fill(0);
    let n = text.len().min(dst.len().saturating_sub(1));
    for (i, b) in text.bytes().take(n).enumerate() {
        dst[i] = b as c_char;
    }
}
pub fn wstring(text: &str, dst: &mut [u16]) {
    dst.fill(0);
    let n = dst.len().saturating_sub(1);
    for (d, s) in dst.iter_mut().take(n).zip(text.encode_utf16()) {
        *d = s;
    }
}

pub struct Plugin {
    pub engine: Arc<Engine>,
    revision: AtomicFloat,
    sample_rate: AtomicFloat,
    initialized: AtomicBool,
    handler: RefCell<Option<ComPtr<IComponentHandler>>>,
    controller_address: AtomicPtr<IEditController>,
}
impl Class for Plugin {
    type Interfaces = (
        IComponent,
        IAudioProcessor,
        IEditController,
        IProcessContextRequirements,
    );
}
impl Plugin {
    fn new() -> Self {
        Self {
            engine: Engine::new(),
            revision: AtomicFloat::new(0.),
            sample_rate: AtomicFloat::new(44100.),
            initialized: AtomicBool::new(false),
            handler: RefCell::new(None),
            controller_address: AtomicPtr::new(ptr::null_mut()),
        }
    }
    pub fn normalized(&self, id: u32) -> f64 {
        let s = &self.engine.shared;
        match id {
            GAIN => s.gain.get(),
            OFFSET => (s.offset.get() + 3600.) / 7200.,
            MUTE => s.muted.load(Ordering::Relaxed) as u8 as f64,
            FILE_REVISION => self.revision.get(),
            _ => 0.,
        }
    }
    fn apply(&self, id: u32, value: f64) -> tresult {
        if !value.is_finite() {
            return kInvalidArgument;
        }
        let v = value.clamp(0., 1.);
        let s = &self.engine.shared;
        match id {
            GAIN => s.gain.set(v),
            OFFSET => s.offset.set(v * 7200. - 3600.),
            MUTE => s.muted.store(v >= 0.5, Ordering::Relaxed),
            FILE_REVISION => self.revision.set(v),
            _ => return kInvalidArgument,
        }
        kResultOk
    }
}
impl IPluginBaseTrait for Plugin {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        if self.initialized.swap(true, Ordering::AcqRel) {
            kResultFalse
        } else {
            kResultOk
        }
    }
    unsafe fn terminate(&self) -> tresult {
        self.initialized.store(false, Ordering::Release);
        self.handler.borrow_mut().take();
        kResultOk
    }
}
impl IComponentTrait for Plugin {
    unsafe fn getControllerClassId(&self, _id: *mut TUID) -> tresult {
        kNotImplemented
    }
    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }
    unsafe fn getBusCount(&self, media: MediaType, dir: BusDirection) -> i32 {
        if (media == MediaTypes_::kAudio as i32 && dir == BusDirections_::kOutput as i32)
            || (media == MediaTypes_::kEvent as i32 && dir == BusDirections_::kInput as i32)
        {
            1
        } else {
            0
        }
    }
    unsafe fn getBusInfo(
        &self,
        media: MediaType,
        dir: BusDirection,
        index: i32,
        bus: *mut BusInfo,
    ) -> tresult {
        if bus.is_null() || index != 0 || self.getBusCount(media, dir) != 1 {
            return kInvalidArgument;
        }
        let b = &mut *bus;
        *b = std::mem::zeroed();
        b.mediaType = media;
        b.direction = dir;
        b.channelCount = if media == MediaTypes_::kAudio as i32 {
            2
        } else {
            16
        };
        b.busType = BusTypes_::kMain as i32;
        b.flags = BusInfo_::BusFlags_::kDefaultActive;
        wstring(
            if media == MediaTypes_::kAudio as i32 {
                "Recording output"
            } else {
                "MIDI (ignored)"
            },
            &mut b.name,
        );
        kResultOk
    }
    unsafe fn getRoutingInfo(
        &self,
        _input: *mut RoutingInfo,
        _output: *mut RoutingInfo,
    ) -> tresult {
        kNotImplemented
    }
    unsafe fn activateBus(
        &self,
        media: MediaType,
        dir: BusDirection,
        index: i32,
        _state: TBool,
    ) -> tresult {
        if index == 0 && self.getBusCount(media, dir) == 1 {
            kResultOk
        } else {
            kInvalidArgument
        }
    }
    unsafe fn setActive(&self, active: TBool) -> tresult {
        if active == 0 {
            self.engine.shared.playing.store(false, Ordering::Relaxed);
        }
        kResultOk
    }
    unsafe fn setState(&self, stream: *mut IBStream) -> tresult {
        let Some(stream) = ComRef::from_raw(stream) else {
            return kInvalidArgument;
        };
        let mut header = [0u8; 32];
        if !read_exact(stream, &mut header) {
            return kResultFalse;
        }
        let length = u32::from_le_bytes(header[28..32].try_into().unwrap()) as usize;
        if length > state::MAX_PATH {
            return kResultFalse;
        }
        let mut data = Vec::from(header);
        data.resize(32 + length, 0);
        if !read_exact(stream, &mut data[32..]) {
            return kResultFalse;
        }
        let Some(saved) = state::decode(&data) else {
            return kResultFalse;
        };
        self.apply(GAIN, saved.gain);
        self.apply(OFFSET, (saved.offset + 3600.) / 7200.);
        self.apply(MUTE, saved.muted as u8 as f64);
        let current = self.engine.metadata();
        if current.path != saved.path || (!current.ready && !current.loading) {
            self.engine.load(saved.path);
        }
        kResultOk
    }
    unsafe fn getState(&self, stream: *mut IBStream) -> tresult {
        let Some(stream) = ComRef::from_raw(stream) else {
            return kInvalidArgument;
        };
        let s = &self.engine.shared;
        let path = self.engine.metadata().path;
        if path.len() > state::MAX_PATH {
            return kResultFalse;
        }
        let bytes = state::encode(&state::Saved {
            gain: s.gain.get(),
            offset: s.offset.get(),
            muted: s.muted.load(Ordering::Relaxed),
            path,
        });
        if write_all(stream, &bytes) {
            kResultOk
        } else {
            kResultFalse
        }
    }
}
unsafe fn read_exact(stream: ComRef<'_, IBStream>, mut bytes: &mut [u8]) -> bool {
    while !bytes.is_empty() {
        let mut count = 0;
        stream.read(bytes.as_mut_ptr().cast(), bytes.len() as i32, &mut count);
        if count <= 0 || count as usize > bytes.len() {
            return false;
        }
        bytes = &mut bytes[count as usize..];
    }
    true
}
unsafe fn write_all(stream: ComRef<'_, IBStream>, mut bytes: &[u8]) -> bool {
    while !bytes.is_empty() {
        let mut count = 0;
        stream.write(
            bytes.as_ptr().cast_mut().cast(),
            bytes.len() as i32,
            &mut count,
        );
        if count <= 0 || count as usize > bytes.len() {
            return false;
        }
        bytes = &bytes[count as usize..];
    }
    true
}
impl IAudioProcessorTrait for Plugin {
    unsafe fn setBusArrangements(
        &self,
        _inputs: *mut SpeakerArrangement,
        ins: i32,
        outputs: *mut SpeakerArrangement,
        outs: i32,
    ) -> tresult {
        if ins == 0 && outs == 1 && !outputs.is_null() && *outputs == SpeakerArr::kStereo {
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn getBusArrangement(
        &self,
        dir: BusDirection,
        index: i32,
        arr: *mut SpeakerArrangement,
    ) -> tresult {
        if dir == BusDirections_::kOutput as i32 && index == 0 && !arr.is_null() {
            *arr = SpeakerArr::kStereo;
            kResultOk
        } else {
            kInvalidArgument
        }
    }
    unsafe fn canProcessSampleSize(&self, size: i32) -> tresult {
        if size == SymbolicSampleSizes_::kSample32 as i32 {
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn getLatencySamples(&self) -> u32 {
        0
    }
    unsafe fn setupProcessing(&self, setup: *mut ProcessSetup) -> tresult {
        let Some(s) = setup.as_ref() else {
            return kInvalidArgument;
        };
        if !s.sampleRate.is_finite()
            || s.sampleRate <= 0.
            || s.maxSamplesPerBlock < 0
            || self.canProcessSampleSize(s.symbolicSampleSize) != kResultOk
        {
            return kInvalidArgument;
        }
        self.sample_rate.set(s.sampleRate);
        kResultOk
    }
    unsafe fn setProcessing(&self, active: TBool) -> tresult {
        if active == 0 {
            self.engine.shared.playing.store(false, Ordering::Relaxed);
        }
        kResultOk
    }
    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        let Some(d) = data.as_mut() else {
            return kInvalidArgument;
        };
        if let Some(changes) = ComRef::from_raw(d.inputParameterChanges) {
            for i in 0..changes.getParameterCount() {
                if let Some(q) = ComRef::from_raw(changes.getParameterData(i)) {
                    let n = q.getPointCount();
                    if n > 0 {
                        let (mut at, mut value) = (0, 0.);
                        if q.getPoint(n - 1, &mut at, &mut value) == kResultOk {
                            self.apply(q.getParameterId(), value);
                        }
                    }
                }
            }
        }
        if d.symbolicSampleSize != SymbolicSampleSizes_::kSample32 as i32 {
            return kResultFalse;
        }
        if d.numSamples < 0 || d.numOutputs < 0 {
            return kInvalidArgument;
        }
        if d.numSamples == 0 || d.numOutputs == 0 {
            return kResultOk;
        }
        let Some(output) = d.outputs.as_mut() else {
            return kInvalidArgument;
        };
        if output.numChannels != 2 || output.__field0.channelBuffers32.is_null() {
            return kInvalidArgument;
        }
        let channels = output.__field0.channelBuffers32;
        let (left, right) = (*channels, *channels.add(1));
        let len = d.numSamples as usize;
        let bytes = len * std::mem::size_of::<f32>();
        if left.is_null() || right.is_null() || (left as usize).abs_diff(right as usize) < bytes {
            return kInvalidArgument;
        }
        let context = d.processContext.as_ref();
        let playing =
            context.is_some_and(|c| c.state & ProcessContext_::StatesAndFlags_::kPlaying != 0);
        let rate = context.map_or(self.sample_rate.get(), |c| {
            if c.sampleRate > 0. {
                c.sampleRate
            } else {
                self.sample_rate.get()
            }
        });
        self.engine.render(
            std::slice::from_raw_parts_mut(left, len),
            std::slice::from_raw_parts_mut(right, len),
            rate,
            context.map_or(0, |c| c.projectTimeSamples),
            playing,
        );
        output.silenceFlags = 0;
        kResultOk
    }
    unsafe fn getTailSamples(&self) -> u32 {
        0
    }
}
impl IProcessContextRequirementsTrait for Plugin {
    unsafe fn getProcessContextRequirements(&self) -> u32 {
        IProcessContextRequirements_::Flags_::kNeedTransportState
    }
}
impl IEditControllerTrait for Plugin {
    unsafe fn setComponentState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn setState(&self, _state: *mut IBStream) -> tresult {
        kNotImplemented
    }
    unsafe fn getState(&self, _state: *mut IBStream) -> tresult {
        kNotImplemented
    }
    unsafe fn getParameterCount(&self) -> i32 {
        4
    }
    unsafe fn getParameterInfo(&self, index: i32, info: *mut ParameterInfo) -> tresult {
        if !(0..4).contains(&index) || info.is_null() {
            return kInvalidArgument;
        }
        let p = &mut *info;
        *p = std::mem::zeroed();
        p.id = index as u32 + 1;
        let (title, unit, default, steps) = match p.id {
            GAIN => ("Recording level", "%", 0.8, 0),
            OFFSET => ("Offset", "s", 0.5, 0),
            MUTE => ("Mute recording", "", 0., 1),
            _ => ("File state", "", 0., 1),
        };
        wstring(title, &mut p.title);
        wstring(title, &mut p.shortTitle);
        wstring(unit, &mut p.units);
        p.defaultNormalizedValue = default;
        p.stepCount = steps;
        p.unitId = 0;
        p.flags = if p.id == FILE_REVISION {
            ParameterInfo_::ParameterFlags_::kIsHidden
        } else {
            ParameterInfo_::ParameterFlags_::kCanAutomate
        };
        kResultOk
    }
    unsafe fn getParamStringByValue(&self, id: u32, value: f64, string: *mut String128) -> tresult {
        if string.is_null() || !(1..=4).contains(&id) || !value.is_finite() {
            return kInvalidArgument;
        }
        wstring(
            &format!("{:0.2}", self.normalizedParamToPlain(id, value)),
            &mut *string,
        );
        kResultOk
    }
    unsafe fn getParamValueByString(
        &self,
        id: u32,
        string: *mut TChar,
        value: *mut f64,
    ) -> tresult {
        if string.is_null() || value.is_null() || !(1..=4).contains(&id) {
            return kInvalidArgument;
        }
        let mut len = 0;
        while len < 128 && *string.add(len) != 0 {
            len += 1;
        }
        if len == 128 {
            return kInvalidArgument;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(string.cast::<u16>(), len));
        if let Ok(plain) = text.trim().parse::<f64>() {
            if plain.is_finite() {
                *value = self.plainParamToNormalized(id, plain).clamp(0., 1.);
                return kResultOk;
            }
        }
        kInvalidArgument
    }
    unsafe fn normalizedParamToPlain(&self, id: u32, value: f64) -> f64 {
        match id {
            GAIN => value * 100.,
            OFFSET => value * 7200. - 3600.,
            _ => value,
        }
    }
    unsafe fn plainParamToNormalized(&self, id: u32, value: f64) -> f64 {
        match id {
            GAIN => value / 100.,
            OFFSET => (value + 3600.) / 7200.,
            _ => value,
        }
    }
    unsafe fn getParamNormalized(&self, id: u32) -> f64 {
        self.normalized(id)
    }
    unsafe fn setParamNormalized(&self, id: u32, value: f64) -> tresult {
        self.apply(id, value)
    }
    unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> tresult {
        *self.handler.borrow_mut() = ComRef::from_raw(handler).map(|h| h.to_com_ptr());
        kResultOk
    }
    unsafe fn createView(&self, name: *const c_char) -> *mut IPlugView {
        if name.is_null() || CStr::from_ptr(name).to_bytes() != b"editor" {
            return ptr::null_mut();
        }
        let Some(controller) = ComRef::from_raw(self.controller_address.load(Ordering::Acquire))
        else {
            return ptr::null_mut();
        };
        editor::create(UiActions {
            engine: self.engine.clone(),
            controller: controller.to_com_ptr(),
            handler: self.handler.borrow().clone(),
        })
    }
}
// A small shared notification endpoint keeps UI file edits and parameter edits
// independent of the processor's realtime callback.
#[derive(Clone)]
pub struct UiActions {
    pub engine: Arc<Engine>,
    pub controller: ComPtr<IEditController>,
    pub handler: Option<ComPtr<IComponentHandler>>,
}
impl UiActions {
    pub fn change(&self, id: u32, value: f64) {
        unsafe {
            if let Some(h) = &self.handler {
                h.beginEdit(id);
            }
            self.controller.setParamNormalized(id, value);
            if let Some(h) = &self.handler {
                h.performEdit(id, value);
                h.endEdit(id);
                if let Some(h2) = h.cast::<IComponentHandler2>() {
                    h2.setDirty(1);
                }
            }
        }
    }
    pub fn load(&self, path: String) {
        self.engine.load(path);
        unsafe {
            let old = self.controller.getParamNormalized(FILE_REVISION);
            self.change(FILE_REVISION, if old >= 0.5 { 0. } else { 1. });
        }
    }
}

struct Factory;
impl Class for Factory {
    type Interfaces = (IPluginFactory2,);
}
impl IPluginFactoryTrait for Factory {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }
        let i = &mut *info;
        *i = std::mem::zeroed();
        cstring("Reference Audio", &mut i.vendor);
        i.flags = PFactoryInfo_::FactoryFlags_::kUnicode as i32;
        kResultOk
    }
    unsafe fn countClasses(&self) -> i32 {
        1
    }
    unsafe fn getClassInfo(&self, index: i32, info: *mut PClassInfo) -> tresult {
        if index != 0 || info.is_null() {
            return kInvalidArgument;
        }
        let i = &mut *info;
        *i = std::mem::zeroed();
        i.cid = CID;
        i.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as i32;
        cstring("Audio Module Class", &mut i.category);
        cstring("Reference Audio", &mut i.name);
        kResultOk
    }
    unsafe fn createInstance(
        &self,
        cid: FIDString,
        iid: FIDString,
        obj: *mut *mut c_void,
    ) -> tresult {
        if cid.is_null() || iid.is_null() || obj.is_null() {
            return kInvalidArgument;
        }
        *obj = ptr::null_mut();
        if ptr::read_unaligned(cid.cast::<TUID>()) != CID {
            return kInvalidArgument;
        }
        let Ok(instance) = std::panic::catch_unwind(|| ComWrapper::new(Plugin::new())) else {
            return kInternalError;
        };
        let Some(controller) = instance.as_com_ref::<IEditController>() else {
            return kInternalError;
        };
        instance
            .controller_address
            .store(controller.as_ptr(), Ordering::Release);
        let Some(unknown) = instance.to_com_ptr::<FUnknown>() else {
            return kInternalError;
        };
        let p = unknown.as_ptr();
        ((*(*p).vtbl).queryInterface)(p, iid.cast::<TUID>(), obj)
    }
}
impl IPluginFactory2Trait for Factory {
    unsafe fn getClassInfo2(&self, index: i32, info: *mut PClassInfo2) -> tresult {
        if index != 0 || info.is_null() {
            return kInvalidArgument;
        }
        let i = &mut *info;
        *i = std::mem::zeroed();
        i.cid = CID;
        i.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as i32;
        cstring("Audio Module Class", &mut i.category);
        cstring("Reference Audio", &mut i.name);
        cstring("Instrument|Other", &mut i.subCategories);
        cstring("Reference Audio", &mut i.vendor);
        cstring(VERSION, &mut i.version);
        cstring("VST 3 / vst3-rs 0.3.0", &mut i.sdkVersion);
        kResultOk
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetPluginFactory() -> *mut IPluginFactory {
    ComWrapper::new(Factory)
        .to_com_ptr::<IPluginFactory>()
        .unwrap()
        .into_raw()
}
#[unsafe(no_mangle)]
pub extern "system" fn bundleEntry(_bundle: *mut c_void) -> bool {
    true
}
#[unsafe(no_mangle)]
pub extern "system" fn bundleExit() -> bool {
    true
}
#[unsafe(no_mangle)]
pub extern "system" fn BundleEntry(bundle: *mut c_void) -> bool {
    bundleEntry(bundle)
}
#[unsafe(no_mangle)]
pub extern "system" fn BundleExit() -> bool {
    bundleExit()
}

pub fn pin_for_native_editor() -> bool {
    // Objective-C classes remain registered for the process lifetime. Pin the
    // image before registering one so host unload/reload cannot leave their
    // method pointers referring to unmapped code. This is done once, off the
    // audio thread; the image is reclaimed when MuseScore exits.
    static PINNED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PINNED.get_or_init(|| unsafe {
        let mut info: libc::Dl_info = std::mem::zeroed();
        libc::dladdr(GetPluginFactory as *const () as *const c_void, &mut info) != 0
            && !info.dli_fname.is_null()
            && !libc::dlopen(info.dli_fname, libc::RTLD_NOW | libc::RTLD_NODELETE).is_null()
    })
}
