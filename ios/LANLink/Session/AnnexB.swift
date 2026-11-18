import Foundation

/// H.264 Annex B byte streams (start-code delimited, as the PC sends them)
/// and the AVCC form (4-byte big-endian lengths) that Core Media expects.
/// The start-code scan matches `parameter_sets` in
/// crates/lanlink-mobile/src/video.rs.
enum AnnexB {
    /// NAL unit types used by the decoder.
    static let NAL_SLICE = 1
    static let NAL_IDR = 5
    static let NAL_SPS = 7
    static let NAL_PPS = 8
    static let NAL_AUD = 9

    /// The NAL units of `data`, without their start codes. Accepts 3- and
    /// 4-byte start codes; empty units and bytes before the first start
    /// code are dropped.
    static func nalUnits(_ data: Data) -> [Data] {
        let b = [UInt8](data)
        let n = b.count
        // (start of the start code, first payload byte)
        var starts: [(Int, Int)] = []
        var i = 0
        while i + 3 <= n {
            if b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 1 {
                starts.append((i, i + 3))
                i += 3
            } else if i + 4 <= n && b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 0 && b[i + 3] == 1 {
                starts.append((i, i + 4))
                i += 4
            } else {
                i += 1
            }
        }
        var out: [Data] = []
        for (k, s) in starts.enumerated() {
            let end = k + 1 < starts.count ? starts[k + 1].0 : n
            if s.1 < end {
                out.append(Data(b[s.1..<end]))
            }
        }
        return out
    }

    /// The units as one AVCC buffer: each prefixed by its 4-byte big-endian
    /// length.
    static func avcc(_ nals: [Data]) -> Data {
        var out = Data(capacity: nals.reduce(0) { $0 + 4 + $1.count })
        for nal in nals {
            let len = UInt32(truncatingIfNeeded: nal.count)
            out.append(contentsOf: [
                UInt8(truncatingIfNeeded: len >> 24), UInt8(truncatingIfNeeded: len >> 16),
                UInt8(truncatingIfNeeded: len >> 8), UInt8(truncatingIfNeeded: len),
            ])
            out.append(nal)
        }
        return out
    }

    /// The first SPS and the first PPS among `nals`, or nil unless both are
    /// present.
    static func parameterSets(_ nals: [Data]) -> (sps: Data, pps: Data)? {
        guard let sps = nals.first(where: { nalType($0) == NAL_SPS }),
              let pps = nals.first(where: { nalType($0) == NAL_PPS })
        else { return nil }
        return (sps, pps)
    }

    /// The NAL unit type (low five bits of the header byte), or -1 if empty.
    static func nalType(_ nal: Data) -> Int {
        guard let header = nal.first else { return -1 }
        return Int(header & 0x1F)
    }
}
