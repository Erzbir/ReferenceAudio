pub const MAGIC: u32 = 0x52415544;
pub const MAX_PATH: usize = 32768;
#[derive(Debug, PartialEq)]
pub struct Saved {
    pub gain: f64,
    pub offset: f64,
    pub muted: bool,
    pub path: String,
}
pub fn encode(s: &Saved) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(32 + s.path.len());
    bytes.extend(MAGIC.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(s.gain.to_le_bytes());
    bytes.extend(s.offset.to_le_bytes());
    bytes.extend((s.muted as i32).to_le_bytes());
    bytes.extend((s.path.len() as u32).to_le_bytes());
    bytes.extend(s.path.as_bytes());
    bytes
}
pub fn decode(bytes: &[u8]) -> Option<Saved> {
    if bytes.len() < 32 {
        return None;
    }
    let u32at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    if u32at(0) != MAGIC || u32at(4) != 1 {
        return None;
    }
    let gain = f64::from_le_bytes(bytes[8..16].try_into().ok()?);
    let offset = f64::from_le_bytes(bytes[16..24].try_into().ok()?);
    let size = u32at(28) as usize;
    if size > MAX_PATH || !gain.is_finite() || !offset.is_finite() || bytes.len() != 32 + size {
        return None;
    }
    let path = std::str::from_utf8(&bytes[32..]).ok()?.to_owned();
    if path.contains('\0') {
        return None;
    }
    Some(Saved {
        gain: gain.clamp(0., 1.),
        offset: offset.clamp(-3600., 3600.),
        muted: u32at(24) != 0,
        path,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_actual_cpp_release_state() {
        let bytes = include_bytes!("../resources/legacy-state.bin");
        let saved = decode(bytes).expect("state saved by the C++ release");
        assert_eq!(saved.gain, 0.8);
        assert_eq!(saved.offset, 0.0);
        assert!(!saved.muted);
        assert!(saved.path.ends_with("/work/follow-test.wav"));
        assert_eq!(encode(&saved), bytes);
    }
    #[test]
    fn roundtrip() {
        let s = Saved {
            gain: 0.8,
            offset: -2.5,
            muted: true,
            path: "/tmp/reference.wav".into(),
        };
        assert_eq!(decode(&encode(&s)), Some(s));
    }
    #[test]
    fn rejects_invalid() {
        let s = Saved {
            gain: 0.8,
            offset: 0.,
            muted: false,
            path: "test.wav".into(),
        };
        let mut b = encode(&s);
        b[4] = 2;
        assert!(decode(&b).is_none());
        assert!(decode(&[]).is_none());
        let mut b = encode(&s);
        b[8..16].copy_from_slice(&f64::NAN.to_le_bytes());
        assert!(decode(&b).is_none());
    }
}
