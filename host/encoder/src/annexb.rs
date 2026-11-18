//! Minimal H.264 Annex B inspection.
//!
//! The encoder output is a byte stream of NAL units separated by start codes
//! (`00 00 01` or `00 00 00 01`). LANLink only needs to know which NAL unit
//! types an access unit contains, to verify that every keyframe is
//! self-contained (SPS + PPS + IDR) before it is sent.

pub const NAL_SLICE: u8 = 1;
pub const NAL_IDR: u8 = 5;
pub const NAL_SEI: u8 = 6;
pub const NAL_SPS: u8 = 7;
pub const NAL_PPS: u8 = 8;
pub const NAL_AUD: u8 = 9;

/// Iterator over the NAL units of an Annex B buffer (payloads without their
/// start codes). Bytes before the first start code are ignored.
pub struct NalUnits<'a> {
    data: &'a [u8],
    pos: usize,
}

pub fn nal_units(data: &[u8]) -> NalUnits<'_> {
    let pos = find_start_code(data, 0)
        .map(|(_, after)| after)
        .unwrap_or(data.len());
    NalUnits { data, pos }
}

/// Find the next start code at or after `from`. Returns the index where it
/// begins and the index just past it.
fn find_start_code(data: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                return Some((i, i + 3));
            }
            if i + 4 <= data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                return Some((i, i + 4));
            }
        }
        i += 1;
    }
    None
}

impl<'a> Iterator for NalUnits<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.pos >= self.data.len() {
            return None;
        }
        let start = self.pos;
        let (end, next) = match find_start_code(self.data, start) {
            Some((begin, after)) => (begin, after),
            None => (self.data.len(), self.data.len()),
        };
        self.pos = next;
        let nal = &self.data[start..end];
        if nal.is_empty() {
            self.next()
        } else {
            Some(nal)
        }
    }
}

/// `nal_unit_type` of a NAL unit (low five bits of the first byte).
pub fn nal_type(nal: &[u8]) -> u8 {
    nal.first().map(|b| b & 0x1f).unwrap_or(0)
}

/// What an access unit contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccessUnitInfo {
    pub has_sps: bool,
    pub has_pps: bool,
    pub has_idr: bool,
    pub has_slice: bool,
    pub nal_count: u32,
}

impl AccessUnitInfo {
    /// A decoder can start from this access unit with no prior state.
    pub fn is_self_contained_keyframe(&self) -> bool {
        self.has_idr && self.has_sps && self.has_pps
    }

    /// Carries picture data at all (not just parameter sets or SEI).
    pub fn has_picture(&self) -> bool {
        self.has_idr || self.has_slice
    }
}

pub fn inspect(data: &[u8]) -> AccessUnitInfo {
    let mut info = AccessUnitInfo::default();
    for nal in nal_units(data) {
        info.nal_count += 1;
        match nal_type(nal) {
            NAL_SPS => info.has_sps = true,
            NAL_PPS => info.has_pps = true,
            NAL_IDR => info.has_idr = true,
            NAL_SLICE => info.has_slice = true,
            _ => {}
        }
    }
    info
}

/// Ensure a keyframe carries its parameter sets by prepending
/// `sequence_header` (SPS + PPS in Annex B form) when they are missing.
/// Returns `true` if the buffer was modified.
pub fn ensure_parameter_sets(data: &mut Vec<u8>, sequence_header: &[u8]) -> bool {
    let info = inspect(data);
    if !info.has_idr || (info.has_sps && info.has_pps) || sequence_header.is_empty() {
        return false;
    }
    let mut out = Vec::with_capacity(sequence_header.len() + data.len());
    out.extend_from_slice(sequence_header);
    out.extend_from_slice(data);
    *data = out;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn au(parts: &[(&[u8], u8)]) -> Vec<u8> {
        let mut v = Vec::new();
        for (start, ty) in parts {
            v.extend_from_slice(start);
            v.push(0x60 | ty); // nal_ref_idc = 3
            v.extend_from_slice(&[0xaa, 0xbb]);
        }
        v
    }

    const S3: &[u8] = &[0, 0, 1];
    const S4: &[u8] = &[0, 0, 0, 1];

    #[test]
    fn splits_mixed_start_codes() {
        let data = au(&[(S4, NAL_SPS), (S3, NAL_PPS), (S4, NAL_IDR)]);
        let types: Vec<u8> = nal_units(&data).map(nal_type).collect();
        assert_eq!(types, [NAL_SPS, NAL_PPS, NAL_IDR]);
        for nal in nal_units(&data) {
            assert_eq!(nal.len(), 3);
        }
    }

    #[test]
    fn classifies_access_units() {
        let key = inspect(&au(&[
            (S4, NAL_AUD),
            (S4, NAL_SPS),
            (S4, NAL_PPS),
            (S3, NAL_IDR),
        ]));
        assert!(key.is_self_contained_keyframe() && key.has_picture());
        assert_eq!(key.nal_count, 4);

        let delta = inspect(&au(&[(S4, NAL_SLICE)]));
        assert!(!delta.is_self_contained_keyframe() && delta.has_picture());

        let bare_idr = inspect(&au(&[(S4, NAL_IDR)]));
        assert!(bare_idr.has_idr && !bare_idr.is_self_contained_keyframe());

        let params_only = inspect(&au(&[(S4, NAL_SPS), (S4, NAL_PPS)]));
        assert!(!params_only.has_picture());
    }

    #[test]
    fn tolerates_garbage_and_edge_cases() {
        assert_eq!(inspect(&[]), AccessUnitInfo::default());
        assert_eq!(inspect(&[0, 0]), AccessUnitInfo::default());
        assert_eq!(inspect(&[0, 0, 1]), AccessUnitInfo::default());
        assert_eq!(inspect(&[1, 2, 3, 4, 5]), AccessUnitInfo::default());
        // Leading junk before the first start code is skipped.
        let mut data = vec![9, 9, 9];
        data.extend(au(&[(S4, NAL_IDR)]));
        assert!(inspect(&data).has_idr);
        // Consecutive start codes yield no empty NAL.
        let doubled = [0, 0, 1, 0, 0, 1, 0x65, 1];
        assert_eq!(nal_units(&doubled).count(), 1);
    }

    #[test]
    fn prepends_parameter_sets_only_when_needed() {
        let header = au(&[(S4, NAL_SPS), (S4, NAL_PPS)]);

        let mut bare = au(&[(S4, NAL_IDR)]);
        assert!(ensure_parameter_sets(&mut bare, &header));
        assert!(inspect(&bare).is_self_contained_keyframe());

        let mut complete = au(&[(S4, NAL_SPS), (S4, NAL_PPS), (S4, NAL_IDR)]);
        let before = complete.clone();
        assert!(!ensure_parameter_sets(&mut complete, &header));
        assert_eq!(complete, before);

        let mut delta = au(&[(S4, NAL_SLICE)]);
        assert!(!ensure_parameter_sets(&mut delta, &header));

        let mut bare2 = au(&[(S4, NAL_IDR)]);
        assert!(!ensure_parameter_sets(&mut bare2, &[]));
    }
}
