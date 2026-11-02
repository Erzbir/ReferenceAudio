use crate::engine::Audio;
use std::ffi::c_void;
use std::ptr;

#[repr(C)]
#[derive(Default)]
struct StreamFormat {
    rate: f64,
    format: u32,
    flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels: u32,
    bits: u32,
    reserved: u32,
}
#[repr(C)]
struct AudioBuffer {
    channels: u32,
    bytes: u32,
    data: *mut c_void,
}
#[repr(C)]
struct AudioBufferList {
    count: u32,
    buffer: AudioBuffer,
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFURLCreateFromFileSystemRepresentation(
        allocator: *const c_void,
        path: *const u8,
        length: isize,
        directory: bool,
    ) -> *const c_void;
    fn CFRelease(object: *const c_void);
}
#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn ExtAudioFileOpenURL(url: *const c_void, file: *mut *mut c_void) -> i32;
    fn ExtAudioFileDispose(file: *mut c_void) -> i32;
    fn ExtAudioFileGetProperty(
        file: *mut c_void,
        property: u32,
        size: *mut u32,
        data: *mut c_void,
    ) -> i32;
    fn ExtAudioFileSetProperty(
        file: *mut c_void,
        property: u32,
        size: u32,
        data: *const c_void,
    ) -> i32;
    fn ExtAudioFileRead(file: *mut c_void, frames: *mut u32, data: *mut AudioBufferList) -> i32;
}
struct File(*mut c_void);
impl Drop for File {
    fn drop(&mut self) {
        unsafe {
            ExtAudioFileDispose(self.0);
        }
    }
}
pub fn decode(path: &str, cancelled: impl Fn() -> bool) -> Result<Audio, String> {
    // SAFETY: The CF URL owns a copy of the validated path. File and URL handles
    // are released on every return path. C structs match Apple's public ABI.
    objc2::rc::autoreleasepool(|_| unsafe {
        let url = CFURLCreateFromFileSystemRepresentation(
            ptr::null(),
            path.as_ptr(),
            path.len() as isize,
            false,
        );
        if url.is_null() {
            return Err("Invalid file path.".into());
        }
        let mut raw = ptr::null_mut();
        let result = ExtAudioFileOpenURL(url, &mut raw);
        CFRelease(url);
        if result != 0 || raw.is_null() {
            return Err(
                "Cannot open audio. Check the file location or use WAV, MP3 or M4A.".into(),
            );
        }
        let file = File(raw);
        let mut source = StreamFormat::default();
        let mut size = std::mem::size_of::<StreamFormat>() as u32;
        if ExtAudioFileGetProperty(
            file.0,
            u32::from_be_bytes(*b"ffmt"),
            &mut size,
            (&mut source as *mut StreamFormat).cast(),
        ) != 0
            || !source.rate.is_finite()
            || source.rate <= 0.
            || source.channels == 0
        {
            return Err("Unrecognized audio format.".into());
        }
        if source.channels > 2 {
            return Err("Convert multichannel audio to mono or stereo first.".into());
        }
        let client = StreamFormat {
            rate: source.rate,
            format: u32::from_be_bytes(*b"lpcm"),
            flags: 1 | 8,
            channels: source.channels,
            bits: 32,
            frames_per_packet: 1,
            bytes_per_packet: 4 * source.channels,
            bytes_per_frame: 4 * source.channels,
            reserved: 0,
        };
        if ExtAudioFileSetProperty(
            file.0,
            u32::from_be_bytes(*b"cfmt"),
            size,
            (&client as *const StreamFormat).cast(),
        ) != 0
        {
            return Err("Audio decoding failed. Try a WAV file.".into());
        }
        let max_samples = 512 * 1024 * 1024 / std::mem::size_of::<f32>();
        let channels = source.channels as usize;
        let mut samples = Vec::new();
        let mut length = 0i64;
        let mut size = 8;
        if ExtAudioFileGetProperty(
            file.0,
            u32::from_be_bytes(*b"#frm"),
            &mut size,
            (&mut length as *mut i64).cast(),
        ) == 0
            && length > 0
        {
            let count = (length as usize)
                .checked_mul(channels)
                .ok_or("Recording is too long.")?;
            if count > max_samples {
                return Err("Decoded audio exceeds 512 MB. Choose a shorter recording.".into());
            }
            samples
                .try_reserve_exact(count)
                .map_err(|_| "Not enough memory. Choose a shorter recording.")?;
        }
        let mut block = vec![0f32; 8192 * channels];
        loop {
            if cancelled() {
                return Err("Loading cancelled.".into());
            }
            let mut frames = 8192;
            let mut list = AudioBufferList {
                count: 1,
                buffer: AudioBuffer {
                    channels: source.channels,
                    bytes: (block.len() * 4) as u32,
                    data: block.as_mut_ptr().cast(),
                },
            };
            if ExtAudioFileRead(file.0, &mut frames, &mut list) != 0 {
                return Err("Audio read failed. The file may be incomplete.".into());
            }
            if frames == 0 {
                break;
            }
            if frames > 8192 {
                return Err("Audio decoder returned an invalid block.".into());
            }
            let count = frames as usize * channels;
            if samples.len() + count > max_samples {
                return Err("Decoded audio exceeds 512 MB. Choose a shorter recording.".into());
            }
            samples
                .try_reserve(count)
                .map_err(|_| "Not enough memory. Choose a shorter recording.")?;
            samples.extend(
                block[..count]
                    .iter()
                    .map(|x| if x.is_finite() { *x } else { 0. }),
            );
        }
        if samples.is_empty() {
            return Err("This file contains no playable audio.".into());
        }
        Ok(Audio {
            rate: source.rate,
            channels,
            samples,
        })
    })
}
