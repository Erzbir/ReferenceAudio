#![allow(unsafe_op_in_unsafe_fn)]
use crate::vst::{UiActions, GAIN, MUTE, OFFSET};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{
    MainThreadMarker, NSArray, NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes,
    NSSize, NSString, NSTimer, NSURL,
};
use std::cell::{OnceCell, RefCell};
use std::ffi::{c_void, CStr};
use std::sync::atomic::Ordering;
use vst3::{Class, ComPtr, ComWrapper, Steinberg::*};

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
fn text(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}
fn clock(seconds: f64) -> String {
    if !seconds.is_finite() {
        return "--:--.--".into();
    }
    let v = seconds.abs();
    format!(
        "{}{:02}:{:05.2}",
        if seconds < 0. { "-" } else { "" },
        (v / 60.) as u64,
        v % 60.
    )
}
struct Controls {
    filename: Retained<NSTextField>,
    status: Retained<NSTextField>,
    score: Retained<NSTextField>,
    audio: Retained<NSTextField>,
    offset: Retained<NSTextField>,
    stepper: Retained<NSStepper>,
    gain: Retained<NSSlider>,
    level: Retained<NSTextField>,
    mute: Retained<NSButton>,
    progress: Retained<NSProgressIndicator>,
}
struct RootIvars {
    actions: UiActions,
    controls: OnceCell<Controls>,
    timer: RefCell<Option<Retained<NSTimer>>>,
}
define_class!(
    // SAFETY: NSView is subclassed on the main thread; overrides use their
    // documented Objective-C signatures and inherited initialization.
    #[unsafe(super=NSView)]
    #[thread_kind=MainThreadOnly]
    #[ivars=RootIvars]
    #[name = "ReferenceAudioRustView"]
    struct Root;
    unsafe impl NSObjectProtocol for Root {}
    impl Root {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool {
            true
        }
        #[unsafe(method(isOpaque))]
        fn opaque(&self) -> bool {
            true
        }
        #[unsafe(method(drawRect:))]
        fn draw(&self, dirty: NSRect) {
            NSColor::colorWithSRGBRed_green_blue_alpha(0.13, 0.14, 0.16, 1.).setFill();
            NSRectFill(dirty);
        }
        #[unsafe(method(refresh:))]
        fn tick(&self, _sender: &NSTimer) {
            self.refresh();
        }
        #[unsafe(method(openAudio:))]
        fn open(&self, _sender: &AnyObject) {
            unsafe {
                let panel = NSOpenPanel::openPanel(self.mtm());
                panel.setTitle(Some(&text("Choose a reference recording")));
                panel.setCanChooseDirectories(false);
                panel.setAllowsMultipleSelection(false);
                panel.setAllowedContentTypes(&NSArray::from_slice(&[
                    objc2_uniform_type_identifiers::UTTypeAudio,
                ]));
                if panel.runModal() == 1 {
                    if let Some(url) = panel.URL() {
                        if url.isFileURL() {
                            if let Some(path) = url.path() {
                                self.ivars().actions.load(path.to_string());
                            }
                        }
                    }
                }
                self.refresh();
            }
        }
        #[unsafe(method(changeOffset:))]
        fn offset_changed(&self, _sender: &AnyObject) {
            if let Some(c) = self.ivars().controls.get() {
                let value = c.offset.doubleValue();
                if value.is_finite() {
                    self.ivars()
                        .actions
                        .change(OFFSET, (value.clamp(-3600., 3600.) + 3600.) / 7200.);
                }
                self.refresh();
            }
        }
        #[unsafe(method(stepOffset:))]
        fn stepped(&self, _sender: &AnyObject) {
            if let Some(c) = self.ivars().controls.get() {
                self.ivars()
                    .actions
                    .change(OFFSET, (c.stepper.doubleValue() + 3600.) / 7200.);
                self.refresh();
            }
        }
        #[unsafe(method(resetOffset:))]
        fn reset(&self, _sender: &AnyObject) {
            self.ivars().actions.change(OFFSET, 0.5);
            self.refresh();
        }
        #[unsafe(method(changeGain:))]
        fn gain_changed(&self, _sender: &AnyObject) {
            if let Some(c) = self.ivars().controls.get() {
                self.ivars().actions.change(GAIN, c.gain.doubleValue());
            }
        }
        #[unsafe(method(changeMute:))]
        fn mute_changed(&self, _sender: &AnyObject) {
            if let Some(c) = self.ivars().controls.get() {
                self.ivars()
                    .actions
                    .change(MUTE, if c.mute.state() == 1 { 1. } else { 0. });
            }
        }
        #[unsafe(method(draggingEntered:))]
        fn dragging(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            if unsafe {
                sender
                    .draggingPasteboard()
                    .stringForType(NSPasteboardTypeFileURL)
            }
            .is_some()
            {
                NSDragOperation::Copy
            } else {
                NSDragOperation::None
            }
        }
        #[unsafe(method(performDragOperation:))]
        fn drop_file(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            unsafe {
                let path = sender
                    .draggingPasteboard()
                    .stringForType(NSPasteboardTypeFileURL)
                    .and_then(|s| NSURL::URLWithString(&s))
                    .filter(|u| u.isFileURL())
                    .and_then(|u| u.path());
                if let Some(path) = path {
                    self.ivars().actions.load(path.to_string());
                    self.refresh();
                    true
                } else {
                    false
                }
            }
        }
    }
);
impl Root {
    unsafe fn new(actions: UiActions, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RootIvars {
            actions,
            controls: OnceCell::new(),
            timer: RefCell::new(None),
        });
        let view: Retained<Self> = msg_send![super(this),initWithFrame:rect(0.,0.,640.,420.)];
        let appearance = NSAppearance::appearanceNamed(NSAppearanceNameDarkAqua);
        view.setAppearance(appearance.as_deref());
        view.registerForDraggedTypes(&NSArray::from_slice(&[NSPasteboardTypeFileURL]));
        view.build();
        view.refresh();
        let timer = NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            0.1,
            &view,
            sel!(refresh:),
            None,
            true,
        );
        NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
        *view.ivars().timer.borrow_mut() = Some(timer);
        view
    }
    fn label(&self, s: &str, x: f64, y: f64, width: f64, size: f64) -> Retained<NSTextField> {
        let label = NSTextField::labelWithString(&text(s), self.mtm());
        label.setFrame(rect(x, y, width, size + 10.));
        label.setFont(Some(&NSFont::systemFontOfSize(size)));
        label.setTextColor(Some(&NSColor::labelColor()));
        self.addSubview(&label);
        label
    }
    unsafe fn button(
        &self,
        s: &str,
        x: f64,
        y: f64,
        width: f64,
        action: objc2::runtime::Sel,
    ) -> Retained<NSButton> {
        let button =
            NSButton::buttonWithTitle_target_action(&text(s), Some(self), Some(action), self.mtm());
        button.setFrame(rect(x, y, width, 32.));
        button.setBezelStyle(NSBezelStyle::Push);
        self.addSubview(&button);
        button
    }
    unsafe fn build(&self) {
        let mtm = self.mtm();
        self.label("Reference Audio", 24., 22., 360., 26.);
        self.label("RECORDING PLAYBACK / MuseScore", 26., 60., 440., 11.)
            .setTextColor(Some(&NSColor::secondaryLabelColor()));
        self.label("Play a recording alongside your score", 26., 99., 400., 15.);
        self.button("Open audio...", 467., 96., 150., sel!(openAudio:));
        let filename = self.label("No recording loaded", 26., 142., 590., 14.);
        let status = self.label("Open a reference recording", 26., 169., 590., 12.);
        self.label("Score time", 26., 216., 260., 11.)
            .setTextColor(Some(&NSColor::secondaryLabelColor()));
        self.label("Recording time", 332., 216., 260., 11.)
            .setTextColor(Some(&NSColor::secondaryLabelColor()));
        let score = self.label("00:00.00", 24., 236., 280., 26.);
        let audio = self.label("00:00.00", 330., 236., 280., 26.);
        score.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(
            26.,
            NSFontWeightRegular,
        )));
        audio.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(
            26.,
            NSFontWeightRegular,
        )));
        let progress = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(mtm),
            rect(26., 281., 588., 5.),
        );
        progress.setIndeterminate(false);
        progress.setMinValue(0.);
        progress.setMaxValue(1.);
        progress.setStyle(NSProgressIndicatorStyle::Bar);
        self.addSubview(&progress);
        self.label("Offset", 26., 309., 92., 13.);
        let offset =
            NSTextField::initWithFrame(NSTextField::alloc(mtm), rect(116., 303., 111., 28.));
        offset.setAlignment(NSTextAlignment::Right);
        offset.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(
            14.,
            NSFontWeightRegular,
        )));
        offset.setTarget(Some(self));
        offset.setAction(Some(sel!(changeOffset:)));
        self.addSubview(&offset);
        let stepper = NSStepper::initWithFrame(NSStepper::alloc(mtm), rect(231., 301., 20., 30.));
        stepper.setMinValue(-3600.);
        stepper.setMaxValue(3600.);
        stepper.setIncrement(0.01);
        stepper.setValueWraps(false);
        stepper.setTarget(Some(self));
        stepper.setAction(Some(sel!(stepOffset:)));
        self.addSubview(&stepper);
        self.label("s", 263., 309., 30., 13.);
        self.button("Reset", 297., 301., 65., sel!(resetOffset:));
        self.label(
            "Positive skips ahead; negative delays",
            374.,
            311.,
            244.,
            11.,
        )
        .setTextColor(Some(&NSColor::secondaryLabelColor()));
        self.label("Level", 26., 357., 92., 13.);
        let gain = NSSlider::sliderWithValue_minValue_maxValue_target_action(
            0.8,
            0.,
            1.,
            Some(self),
            Some(sel!(changeGain:)),
            mtm,
        );
        gain.setFrame(rect(113., 350., 261., 28.));
        gain.setContinuous(true);
        self.addSubview(&gain);
        let level = self.label("80%", 388., 357., 65., 13.);
        let mute = NSButton::checkboxWithTitle_target_action(
            &text("Mute recording"),
            Some(self),
            Some(sel!(changeMute:)),
            mtm,
        );
        mute.setFrame(rect(496., 350., 120., 28.));
        self.addSubview(&mute);
        self.label(
            "Use MuseScore playback controls. Settings are saved with the score.",
            26.,
            396.,
            590.,
            11.,
        )
        .setTextColor(Some(&NSColor::secondaryLabelColor()));
        let _ = self.ivars().controls.set(Controls {
            filename,
            status,
            score,
            audio,
            offset,
            stepper,
            gain,
            level,
            mute,
            progress,
        });
    }
    fn refresh(&self) {
        let Some(c) = self.ivars().controls.get() else {
            return;
        };
        let e = &self.ivars().actions.engine;
        let metadata = e.metadata();
        let name = std::path::Path::new(&metadata.path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        c.filename.setStringValue(&text(if name.is_empty() {
            "Drop an audio file here or use Open audio..."
        } else {
            &name
        }));
        c.filename.setToolTip(Some(&text(&metadata.path)));
        let playing = e.recently_playing();
        let status = if metadata.ready {
            format!(
                "{}    Duration {}",
                if playing {
                    "Playing with score"
                } else {
                    "Ready"
                },
                clock(metadata.duration)
            )
        } else {
            metadata.message
        };
        c.status.setStringValue(&text(&status));
        c.status.setToolTip(Some(&text(&status)));
        let color = if metadata.ready && playing {
            NSColor::systemGreenColor()
        } else if !metadata.ready && !metadata.loading && !metadata.path.is_empty() {
            NSColor::systemRedColor()
        } else {
            NSColor::secondaryLabelColor()
        };
        c.status.setTextColor(Some(&color));
        let score = e.shared.score_time.get();
        let offset = e.shared.offset.get();
        let audio = score + offset;
        c.score.setStringValue(&text(&clock(score)));
        c.audio.setStringValue(&text(&clock(audio)));
        c.progress.setDoubleValue(if metadata.duration > 0. {
            (audio / metadata.duration).clamp(0., 1.)
        } else {
            0.
        });
        if c.offset.currentEditor().is_none() {
            c.offset.setStringValue(&text(&format!("{offset:0.2}")));
        }
        c.stepper.setDoubleValue(offset);
        let gain = e.shared.gain.get();
        c.gain.setDoubleValue(gain);
        c.level
            .setStringValue(&text(&format!("{:0.0}%", gain * 100.)));
        c.mute.setState(if e.shared.muted.load(Ordering::Relaxed) {
            1
        } else {
            0
        });
    }
    fn stop(&self) {
        if let Some(timer) = self.ivars().timer.borrow_mut().take() {
            timer.invalidate();
        }
    }
}

struct Editor {
    actions: UiActions,
    view: RefCell<Option<Retained<Root>>>,
    frame: RefCell<Option<ComPtr<IPlugFrame>>>,
}
impl Class for Editor {
    type Interfaces = (IPlugView,);
}
impl Editor {
    fn detach(&self) {
        if let Some(view) = self.view.borrow_mut().take() {
            if MainThreadMarker::new().is_some() {
                view.stop();
                view.removeFromSuperview();
            } else {
                // Transfer the existing +1 reference to the main queue. The
                // image is pinned before a native view is created, so both the
                // callback and Objective-C class remain valid until it runs.
                let address = Retained::into_raw(view) as usize;
                dispatch2::DispatchQueue::main().exec_async(move || {
                    objc2::rc::autoreleasepool(|_| {
                        // SAFETY: The retained reference was transferred once;
                        // only this main-queue callback adopts and releases it.
                        if let Some(view) = unsafe { Retained::from_raw(address as *mut Root) } {
                            view.stop();
                            view.removeFromSuperview();
                        }
                    });
                });
            }
        }
    }
}
impl Drop for Editor {
    fn drop(&mut self) {
        self.detach();
    }
}
impl IPlugViewTrait for Editor {
    unsafe fn isPlatformTypeSupported(&self, kind: FIDString) -> tresult {
        if !kind.is_null() && CStr::from_ptr(kind).to_bytes() == b"NSView" {
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn attached(&self, parent: *mut c_void, kind: FIDString) -> tresult {
        if parent.is_null() || self.isPlatformTypeSupported(kind) != kResultOk {
            return kInvalidArgument;
        }
        let Some(mtm) = MainThreadMarker::new() else {
            return kResultFalse;
        };
        if !crate::vst::pin_for_native_editor() {
            return kResultFalse;
        }
        self.detach();
        let view = Root::new(self.actions.clone(), mtm);
        (&*parent.cast::<NSView>()).addSubview(&view);
        *self.view.borrow_mut() = Some(view);
        kResultOk
    }
    unsafe fn removed(&self) -> tresult {
        if MainThreadMarker::new().is_none() {
            return kResultFalse;
        }
        self.detach();
        kResultOk
    }
    unsafe fn onWheel(&self, _distance: f32) -> tresult {
        kResultFalse
    }
    unsafe fn onKeyDown(&self, _key: char16, _code: int16, _modifiers: int16) -> tresult {
        kResultFalse
    }
    unsafe fn onKeyUp(&self, _key: char16, _code: int16, _modifiers: int16) -> tresult {
        kResultFalse
    }
    unsafe fn getSize(&self, size: *mut ViewRect) -> tresult {
        if size.is_null() {
            return kInvalidArgument;
        }
        *size = ViewRect {
            left: 0,
            top: 0,
            right: 640,
            bottom: 420,
        };
        kResultOk
    }
    unsafe fn onSize(&self, size: *mut ViewRect) -> tresult {
        if size.is_null() {
            return kInvalidArgument;
        }
        let s = &*size;
        if s.right - s.left == 640 && s.bottom - s.top == 420 {
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn onFocus(&self, _state: TBool) -> tresult {
        kResultFalse
    }
    unsafe fn setFrame(&self, frame: *mut IPlugFrame) -> tresult {
        *self.frame.borrow_mut() = vst3::ComRef::from_raw(frame).map(|f| f.to_com_ptr());
        kResultOk
    }
    unsafe fn canResize(&self) -> tresult {
        kResultFalse
    }
    unsafe fn checkSizeConstraint(&self, size: *mut ViewRect) -> tresult {
        if size.is_null() {
            return kInvalidArgument;
        }
        (*size).right = (*size).left + 640;
        (*size).bottom = (*size).top + 420;
        kResultOk
    }
}
pub fn create(actions: UiActions) -> *mut IPlugView {
    ComWrapper::new(Editor {
        actions,
        view: RefCell::new(None),
        frame: RefCell::new(None),
    })
    .to_com_ptr::<IPlugView>()
    .unwrap()
    .into_raw()
}
