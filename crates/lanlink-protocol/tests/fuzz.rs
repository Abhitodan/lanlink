//! Property tests: decoders never panic on arbitrary input, and every value
//! an encoder can produce decodes back to itself.

use lanlink_protocol::control::*;
use lanlink_protocol::error::{ErrorCode, NoticeCode};
use lanlink_protocol::input::*;
use lanlink_protocol::video::*;
use proptest::prelude::*;

fn arb_string(max: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<char>(), 0..max / 4)
        .prop_map(|v| v.into_iter().filter(|c| *c != '\0').collect())
}

fn arb_params() -> impl Strategy<Value = StreamParams> {
    (
        0u8..4,
        0u16..=8192,
        0u16..=8192,
        0u16..=240,
        any::<u32>(),
        any::<u32>(),
    )
        .prop_map(|(q, w, h, fps, b, mb)| StreamParams {
            quality: QualityMode::from_u8(q).unwrap(),
            codec: Codec::H264,
            max_width: w,
            max_height: h,
            fps,
            bitrate_kbps: b,
            max_bitrate_kbps: mb,
        })
}

fn arb_control() -> impl Strategy<Value = ControlMsg> {
    prop_oneof![
        (
            0u16..4,
            0u16..4,
            arb_string(64),
            arb_string(64),
            arb_string(32)
        )
            .prop_map(|(a, b, n, m, v)| ControlMsg::Hello {
                protocol_min: a.min(b),
                protocol_max: a.max(b),
                device_name: n,
                device_model: m,
                app_version: v,
            }),
        proptest::collection::vec(any::<u8>(), 0..64)
            .prop_map(|v| ControlMsg::PairRequest { spake_msg: v }),
        any::<[u8; 32]>().prop_map(|mac| ControlMsg::PairConfirm { mac }),
        (any::<bool>(), any::<[u8; 32]>()).prop_map(|(accepted, mac)| ControlMsg::PairResult {
            accepted,
            mac,
            reason: ErrorCode::PairingDenied,
        }),
        (any::<u32>(), arb_params(), any::<bool>()).prop_map(|(d, p, v)| {
            ControlMsg::SessionStart {
                display_id: d,
                params: p,
                view_only: v,
            }
        }),
        Just(ControlMsg::KeyframeRequest),
        (any::<u32>(), any::<u64>()).prop_map(|(id, t)| ControlMsg::Ping { id, t_send_us: t }),
        (any::<u32>(), any::<u64>(), any::<u64>(), any::<u64>()).prop_map(|(id, a, b, c)| {
            ControlMsg::Pong {
                id,
                t_send_us: a,
                t_recv_us: b,
                t_reply_us: c,
            }
        }),
        arb_string(256).prop_map(|m| ControlMsg::Notice {
            code: NoticeCode::CaptureRecovering,
            message: m,
        }),
        arb_string(256).prop_map(|m| ControlMsg::Error {
            code: ErrorCode::HostBusy,
            message: m,
        }),
        any::<bool>().prop_map(|active| ControlMsg::TextInput { active }),
        (
            any::<bool>(),
            any::<u16>(),
            any::<u16>(),
            any::<u16>(),
            any::<u16>()
        )
            .prop_map(|(visible, x, y, w, h)| ControlMsg::Caret {
                visible,
                x,
                y,
                w,
                h
            }),
        (1u16..=16, 1u16..=16, any::<u32>()).prop_map(|(w, h, id)| ControlMsg::CursorShape {
            shape_id: id,
            width: w,
            height: h,
            hot_x: w - 1,
            hot_y: 0,
            rgba: vec![0x7f; w as usize * h as usize * 4],
        }),
    ]
}

fn arb_input() -> impl Strategy<Value = InputEvent> {
    let kind = prop_oneof![
        (
            0usize..5,
            any::<bool>(),
            proptest::option::of(any::<(u16, u16)>())
        )
            .prop_map(|(b, down, pos)| InputKind::Button {
                button: MouseButton::ALL[b],
                down,
                pos,
            }),
        any::<(i16, i16)>().prop_map(|(dx, dy)| InputKind::Scroll { dx, dy }),
        (1u16..=0xFE, any::<bool>()).prop_map(|(vk, down)| InputKind::Key { vk, down }),
        arb_string(1024).prop_map(|text| InputKind::Text { text }),
        Just(InputKind::ReleaseAll),
        any::<(u16, u16)>().prop_map(|(x, y)| InputKind::MoveAbs { x, y }),
        any::<(i16, i16)>().prop_map(|(dx, dy)| InputKind::MoveRel { dx, dy }),
    ];
    (any::<u32>(), any::<u32>(), kind).prop_map(|(session_id, seq, kind)| InputEvent {
        session_id,
        seq,
        kind,
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn control_decode_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = ControlMsg::decode_body(&bytes);
    }

    #[test]
    fn input_decode_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..128)) {
        let _ = InputEvent::decode_body(&bytes);
    }

    #[test]
    fn datagram_decode_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
        let _ = Datagram::decode(&bytes);
    }

    #[test]
    fn video_headers_never_panic(a in any::<[u8; CONFIG_BODY_LEN]>(), b in any::<[u8; FRAME_HEADER_LEN]>()) {
        let _ = VideoConfig::decode_body(&a);
        if let Ok(h) = FrameHeader::decode_body(&b) {
            // Anything accepted is within the allocation limit.
            prop_assert!(h.payload_len as usize <= lanlink_protocol::limits::MAX_VIDEO_FRAME);
            prop_assert!(h.payload_len > 0);
        }
    }

    #[test]
    fn control_roundtrip(msg in arb_control()) {
        let body = msg.encode_body();
        prop_assert!(body.len() <= lanlink_protocol::limits::MAX_CONTROL_MSG);
        prop_assert_eq!(ControlMsg::decode_body(&body), Ok(msg));
    }

    #[test]
    fn input_roundtrip(ev in arb_input()) {
        let framed = ev.encode_framed();
        let len = u16::from_le_bytes([framed[0], framed[1]]) as usize;
        prop_assert_eq!(len, framed.len() - 2);
        prop_assert!(len <= lanlink_protocol::limits::MAX_INPUT_MSG);
        prop_assert_eq!(InputEvent::decode_body(&framed[2..]), Ok(ev));
    }

    /// Flipping any single byte of a valid message either fails to decode or
    /// decodes to a *different valid* message; it never panics and never
    /// yields something that re-encodes to different bytes than it came from.
    #[test]
    fn control_mutation_is_safe(msg in arb_control(), idx in any::<prop::sample::Index>(), bit in 0u8..8) {
        let mut body = msg.encode_body();
        let i = idx.index(body.len());
        body[i] ^= 1 << bit;
        if let Ok(decoded) = ControlMsg::decode_body(&body) {
            // Canonical encoding: what we accept, we would also emit.
            // (Error/notice codes from the future collapse to a known code,
            // so those two message kinds are exempt.)
            if !matches!(decoded, ControlMsg::Error { .. } | ControlMsg::Notice { .. } | ControlMsg::PairResult { .. } | ControlMsg::SessionEnd { .. }) {
                prop_assert_eq!(decoded.encode_body(), body);
            }
        }
    }

    #[test]
    fn gate_never_decodes_delta_without_reference(
        seqs in proptest::collection::vec((1u32..4, any::<bool>()), 1..200)
    ) {
        // Model: frames arrive with arbitrary gaps and keyframe flags. The
        // gate must only pass a delta frame when the previous sequence number
        // was passed too.
        let mut gate = FrameGate::new();
        gate.on_config(1);
        let mut seq = 0u32;
        let mut prev_decoded: Option<u32> = None;
        for (step, key) in seqs {
            seq += step;
            let h = FrameHeader {
                seq, config_id: 1,
                flags: if key { FLAG_KEYFRAME } else { 0 },
                capture_us: 0, encode_us: 0, send_us: 0, payload_len: 1,
            };
            match gate.on_frame(&h) {
                GateDecision::Decode => {
                    if !key {
                        prop_assert_eq!(prev_decoded, Some(seq - 1));
                    }
                    prev_decoded = Some(seq);
                }
                GateDecision::Skip { .. } => {}
            }
        }
    }
}
