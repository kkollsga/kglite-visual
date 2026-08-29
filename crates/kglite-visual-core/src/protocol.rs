//! Protocol v1 framing (plan D4) — discrete framed binary messages.
//!
//! **Transport-agnostic.** Nothing here knows about WebSockets. A frame is a
//! `Vec<u8>` a caller hands to whatever moves bytes: `Message::Binary` today,
//! an anywidget comm buffer in P5 (D8), a desktop shell later. Message-oriented
//! rather than a byte stream is the reason that stays true — a stream framing
//! would have to be re-implemented on every transport that already delivers
//! discrete messages.
//!
//! ## Frame layout
//!
//! ```text
//! byte 0    4    8    12   16   20   24
//!      |v___|type|seq_|flag|len_|off_|payload… (padded to a multiple of 4)
//! ```
//!
//! Six `u32` little-endian header words, then the payload:
//!
//! | word | field            | meaning                                              |
//! |------|------------------|------------------------------------------------------|
//! | 0    | `version`        | [`PROTOCOL_VERSION`]. **First**, so a mismatched peer  |
//! |      |                  | is diagnosed before any other word is interpreted.     |
//! | 1    | `msg_type`       | [`MessageType`] discriminant.                          |
//! | 2    | `seq`            | frame index within one response, from 0.               |
//! | 3    | `flags`          | bit 0 = [`FLAG_TERMINAL`]: last frame of the response. |
//! | 4    | `payload_len`    | payload bytes in **this** frame, before padding.       |
//! | 5    | `payload_offset` | byte offset of this chunk inside its logical array.    |
//!
//! `payload_offset` is what makes a chunked array self-describing: a decoder
//! places a chunk from its own header rather than from arrival order, so an
//! out-of-order or retried frame cannot silently concatenate wrong.
//!
//! The header is 24 bytes and every frame is padded to a multiple of 4, so a
//! payload always starts 4-byte aligned and stays 4-byte aligned — the
//! precondition for a zero-copy `Float32Array` view on the browser side, which
//! throws on an unaligned `byteOffset` instead of silently copying.

use std::fmt;

/// Wire-format version. A change to any layout rule above changes this number,
/// and the mismatch is a loud decode failure on both sides
/// ([`ProtocolError::VersionMismatch`]) — never a best-effort parse.
pub const PROTOCOL_VERSION: u32 = 1;

/// Header size in bytes (6 × `u32`).
pub const HEADER_BYTES: usize = 24;

/// `flags` bit 0: this is the last frame of the response.
pub const FLAG_TERMINAL: u32 = 1;

/// Every flag bit this version defines. An unknown bit is a decode error, not
/// something to ignore: silently dropping a flag a newer peer set is how a
/// version skew turns into wrong output instead of an error.
const KNOWN_FLAGS: u32 = FLAG_TERMINAL;

/// Target payload size for a chunked array, in bytes.
///
/// The plan's window is 256 KB–1 MB (D4); this sits mid-range and is an exact
/// multiple of the 8-byte record size of both array payloads (a point is two
/// `f32`, a link is two `f32`), so a chunk boundary never splits a record.
pub const CHUNK_TARGET_BYTES: usize = 512 * 1024;

/// Bytes per `f32` array record — one point (x, y) or one link (src, tgt).
const RECORD_BYTES: usize = 8;

/// What a frame's payload is.
///
/// The discriminants are the wire values; they are frozen for
/// [`PROTOCOL_VERSION`] 1 and mirrored into TypeScript by
/// `tests/protocol_baseline.rs`, which `make check-generated-ts` guards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MessageType {
    /// UTF-8 JSON: the meta-graph's metadata half (`MetaGraphMeta`).
    MetaGraphMeta = 1,
    /// `f32` pairs `[x0, y0, x1, y1, …]`, one per slot, in slot order.
    Points = 2,
    /// `f32` pairs `[src0, tgt0, src1, tgt1, …]` of slot indices, ready for
    /// cosmos.gl's `setLinks`. Indices are exact in `f32` up to 2^24.
    Links = 3,
    /// UTF-8 JSON: `SessionInfo`.
    SessionInfo = 4,
    /// UTF-8 JSON: a server-side failure, reported in-band so a client shows
    /// the reason instead of a silent empty view.
    Error = 5,
}

impl MessageType {
    /// Wire value.
    pub const fn code(self) -> u32 {
        self as u32
    }

    /// Every variant, in wire order — the one list the TypeScript mirror and
    /// the decoder are both generated from, so a new variant cannot be added
    /// to one and forgotten in the other.
    pub const ALL: [MessageType; 5] = [
        MessageType::MetaGraphMeta,
        MessageType::Points,
        MessageType::Links,
        MessageType::SessionInfo,
        MessageType::Error,
    ];

    /// The TypeScript constant name for this variant.
    pub const fn ts_name(self) -> &'static str {
        match self {
            MessageType::MetaGraphMeta => "META_GRAPH_META",
            MessageType::Points => "POINTS",
            MessageType::Links => "LINKS",
            MessageType::SessionInfo => "SESSION_INFO",
            MessageType::Error => "ERROR",
        }
    }

    fn from_code(code: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.code() == code)
    }
}

/// A frame that failed to decode.
///
/// Every variant is a refusal. There is deliberately no "recovered" or
/// "ignored" outcome: a decoder that guesses past a header it does not
/// understand produces a wrong picture instead of an error message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// Fewer bytes than a header.
    TooShort { got: usize },
    /// The peer speaks a different wire format. Checked before anything else
    /// in the header is interpreted.
    VersionMismatch { expected: u32, found: u32 },
    /// A `msg_type` word this version does not define.
    UnknownMessageType(u32),
    /// A `flags` bit this version does not define.
    UnknownFlags(u32),
    /// `payload_len` disagrees with the bytes actually present.
    LengthMismatch { declared: usize, available: usize },
    /// The 0–3 padding bytes after the payload were not zero, or were the
    /// wrong count. A frame whose length is not a multiple of 4 breaks the
    /// alignment guarantee every later frame in the same buffer relies on.
    BadPadding,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { got } => {
                write!(f, "frame is {got} bytes, shorter than the {HEADER_BYTES}-byte header")
            }
            Self::VersionMismatch { expected, found } => write!(
                f,
                "protocol version mismatch: this build speaks v{expected}, the frame declares v{found}"
            ),
            Self::UnknownMessageType(code) => write!(f, "unknown message type {code}"),
            Self::UnknownFlags(flags) => write!(f, "unknown flag bits set: {flags:#x}"),
            Self::LengthMismatch { declared, available } => write!(
                f,
                "frame declares a {declared}-byte payload but carries {available}"
            ),
            Self::BadPadding => write!(f, "frame is not padded to a 4-byte boundary with zeros"),
        }
    }
}

impl std::error::Error for ProtocolError {}

/// One decoded frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    pub msg_type: MessageType,
    pub seq: u32,
    pub terminal: bool,
    pub payload_offset: u32,
    pub payload: Vec<u8>,
}

fn read_u32(bytes: &[u8], word: usize) -> u32 {
    let at = word * 4;
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Decode one frame.
///
/// The version word is read and checked **before** any other field, which is
/// the whole reason it is first on the wire: a v2 frame with a re-ordered
/// header must fail as a version mismatch, not as a garbled message type.
pub fn decode_frame(bytes: &[u8]) -> Result<DecodedFrame, ProtocolError> {
    if bytes.len() < HEADER_BYTES {
        return Err(ProtocolError::TooShort { got: bytes.len() });
    }
    let version = read_u32(bytes, 0);
    if version != PROTOCOL_VERSION {
        return Err(ProtocolError::VersionMismatch {
            expected: PROTOCOL_VERSION,
            found: version,
        });
    }

    let msg_code = read_u32(bytes, 1);
    let msg_type =
        MessageType::from_code(msg_code).ok_or(ProtocolError::UnknownMessageType(msg_code))?;
    let seq = read_u32(bytes, 2);
    let flags = read_u32(bytes, 3);
    if flags & !KNOWN_FLAGS != 0 {
        return Err(ProtocolError::UnknownFlags(flags & !KNOWN_FLAGS));
    }
    let payload_len = read_u32(bytes, 4) as usize;
    let payload_offset = read_u32(bytes, 5);

    let available = bytes.len() - HEADER_BYTES;
    if payload_len > available {
        return Err(ProtocolError::LengthMismatch {
            declared: payload_len,
            available,
        });
    }
    let padding = &bytes[HEADER_BYTES + payload_len..];
    if padding.len() >= 4 || !padding.iter().all(|b| *b == 0) {
        return Err(ProtocolError::BadPadding);
    }

    Ok(DecodedFrame {
        msg_type,
        seq,
        terminal: flags & FLAG_TERMINAL != 0,
        payload_offset,
        payload: bytes[HEADER_BYTES..HEADER_BYTES + payload_len].to_vec(),
    })
}

/// Builds the frame sequence for one response.
///
/// Frames are accumulated, then [`finish`](ResponseEncoder::finish) marks the
/// last one terminal. That ordering is deliberate: the terminal flag is a
/// property of the *response*, and a builder that had to be told in advance
/// which push was the last would put that knowledge in every call site.
#[derive(Debug, Default)]
pub struct ResponseEncoder {
    frames: Vec<Vec<u8>>,
}

impl ResponseEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a JSON payload as a single frame. JSON metadata is small by
    /// construction (it is O(#types), never O(V)), so it is never chunked.
    pub fn push_json(&mut self, msg_type: MessageType, json: &str) {
        self.push_frame(msg_type, json.as_bytes(), 0);
    }

    /// Append an `f32` array, chunked at [`CHUNK_TARGET_BYTES`].
    ///
    /// An empty array still emits one frame: "zero points" is an answer, and a
    /// response that simply omits the message leaves a client waiting.
    pub fn push_f32(&mut self, msg_type: MessageType, values: &[f32]) {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        if bytes.is_empty() {
            self.push_frame(msg_type, &[], 0);
            return;
        }
        for (chunk_index, chunk) in bytes.chunks(chunk_bytes()).enumerate() {
            let offset = (chunk_index * chunk_bytes()) as u32;
            self.push_frame(msg_type, chunk, offset);
        }
    }

    fn push_frame(&mut self, msg_type: MessageType, payload: &[u8], payload_offset: u32) {
        let seq = self.frames.len() as u32;
        let padding = (4 - payload.len() % 4) % 4;
        let mut frame = Vec::with_capacity(HEADER_BYTES + payload.len() + padding);
        frame.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        frame.extend_from_slice(&msg_type.code().to_le_bytes());
        frame.extend_from_slice(&seq.to_le_bytes());
        frame.extend_from_slice(&0u32.to_le_bytes()); // flags; finish() sets terminal
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(&payload_offset.to_le_bytes());
        frame.extend_from_slice(payload);
        frame.extend(std::iter::repeat_n(0u8, padding));
        self.frames.push(frame);
    }

    /// Finish the response, marking the last frame terminal.
    pub fn finish(mut self) -> Vec<Vec<u8>> {
        if let Some(last) = self.frames.last_mut() {
            let flags = FLAG_TERMINAL.to_le_bytes();
            last[12..16].copy_from_slice(&flags);
        }
        self.frames
    }
}

/// Chunk size rounded down to a whole number of 8-byte records, so a chunk
/// boundary never splits an (x, y) pair or a (src, tgt) link.
const fn chunk_bytes() -> usize {
    CHUNK_TARGET_BYTES - CHUNK_TARGET_BYTES % RECORD_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_frame_round_trips() {
        let frames = {
            let mut enc = ResponseEncoder::new();
            enc.push_json(MessageType::SessionInfo, r#"{"a":1}"#);
            enc.finish()
        };
        assert_eq!(frames.len(), 1);
        let decoded = decode_frame(&frames[0]).expect("round trip");
        assert_eq!(decoded.msg_type, MessageType::SessionInfo);
        assert_eq!(decoded.seq, 0);
        assert!(decoded.terminal);
        assert_eq!(decoded.payload, br#"{"a":1}"#);
    }

    #[test]
    fn every_frame_is_four_byte_aligned() {
        // A 7-byte JSON payload is the interesting case: the frame is only
        // valid if it is padded to 32 bytes, and every later frame in the same
        // buffer inherits that alignment.
        let mut enc = ResponseEncoder::new();
        enc.push_json(MessageType::Error, r#"{"e":1}"#);
        enc.push_f32(MessageType::Points, &[1.0, 2.0]);
        for frame in enc.finish() {
            assert_eq!(frame.len() % 4, 0, "frame length must be a multiple of 4");
        }
    }

    #[test]
    fn f32_payload_round_trips_bit_exactly() {
        let values: Vec<f32> = (0..1000).map(|i| i as f32 * 0.5).collect();
        let mut enc = ResponseEncoder::new();
        enc.push_f32(MessageType::Points, &values);
        let frames = enc.finish();
        assert_eq!(frames.len(), 1, "4 KB fits in one chunk");
        let decoded = decode_frame(&frames[0]).unwrap();
        let got: Vec<f32> = decoded
            .payload
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        assert_eq!(got, values);
    }

    #[test]
    fn a_large_array_chunks_and_reassembles_by_offset() {
        // 200_000 points = 1.6 MB — more than three chunks, so both the
        // boundary arithmetic and the terminal flag are exercised.
        let values: Vec<f32> = (0..400_000).map(|i| i as f32).collect();
        let mut enc = ResponseEncoder::new();
        enc.push_f32(MessageType::Points, &values);
        let frames = enc.finish();
        assert!(
            frames.len() > 3,
            "expected several chunks, got {}",
            frames.len()
        );

        let mut reassembled = vec![0u8; values.len() * 4];
        for (i, frame) in frames.iter().enumerate() {
            let d = decode_frame(frame).unwrap();
            assert_eq!(d.seq, i as u32);
            assert_eq!(d.terminal, i == frames.len() - 1);
            assert_eq!(
                d.payload.len() % RECORD_BYTES,
                0,
                "a chunk never splits a record"
            );
            let at = d.payload_offset as usize;
            reassembled[at..at + d.payload.len()].copy_from_slice(&d.payload);
        }
        let got: Vec<f32> = reassembled
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        assert_eq!(got, values);
    }

    #[test]
    fn an_empty_array_still_emits_a_frame() {
        let mut enc = ResponseEncoder::new();
        enc.push_f32(MessageType::Links, &[]);
        let frames = enc.finish();
        assert_eq!(frames.len(), 1);
        let d = decode_frame(&frames[0]).unwrap();
        assert!(d.payload.is_empty());
        assert!(d.terminal);
    }

    #[test]
    fn a_version_mismatch_fails_decode_loudly() {
        // The skew test L2 names: a frame from a peer speaking a different
        // wire format must be REFUSED, not parsed on a best-effort basis.
        let mut enc = ResponseEncoder::new();
        enc.push_json(MessageType::SessionInfo, "{}");
        let mut frame = enc.finish().remove(0);
        frame[0..4].copy_from_slice(&(PROTOCOL_VERSION + 1).to_le_bytes());

        let err = decode_frame(&frame).expect_err("a foreign version must not decode");
        assert_eq!(
            err,
            ProtocolError::VersionMismatch {
                expected: PROTOCOL_VERSION,
                found: PROTOCOL_VERSION + 1,
            }
        );
        assert!(
            err.to_string().contains("protocol version mismatch"),
            "the message a user sees must name the cause: {err}"
        );
    }

    #[test]
    fn a_version_mismatch_wins_over_every_other_defect() {
        // Version is read first precisely so a v2 frame whose header was
        // re-ordered reports the version, not the garbage that re-ordering
        // made of the other words.
        let mut frame = vec![0u8; HEADER_BYTES];
        frame[0..4].copy_from_slice(&99u32.to_le_bytes());
        frame[4..8].copy_from_slice(&4242u32.to_le_bytes()); // nonsense msg_type
        frame[12..16].copy_from_slice(&0xffff_ffffu32.to_le_bytes()); // nonsense flags
        assert_eq!(
            decode_frame(&frame),
            Err(ProtocolError::VersionMismatch {
                expected: PROTOCOL_VERSION,
                found: 99
            })
        );
    }

    #[test]
    fn malformed_frames_are_refused() {
        assert_eq!(
            decode_frame(&[0u8; 8]),
            Err(ProtocolError::TooShort { got: 8 })
        );

        let base = {
            let mut enc = ResponseEncoder::new();
            enc.push_json(MessageType::SessionInfo, "{}");
            enc.finish().remove(0)
        };

        let mut bad_type = base.clone();
        bad_type[4..8].copy_from_slice(&77u32.to_le_bytes());
        assert_eq!(
            decode_frame(&bad_type),
            Err(ProtocolError::UnknownMessageType(77))
        );

        let mut bad_flags = base.clone();
        bad_flags[12..16].copy_from_slice(&0b110u32.to_le_bytes());
        assert_eq!(
            decode_frame(&bad_flags),
            Err(ProtocolError::UnknownFlags(0b110))
        );

        let mut long_len = base.clone();
        long_len[16..20].copy_from_slice(&999u32.to_le_bytes());
        assert_eq!(
            decode_frame(&long_len),
            Err(ProtocolError::LengthMismatch {
                declared: 999,
                available: 4 // 2 payload bytes plus the 2 pad bytes
            })
        );

        let mut unpadded = base.clone();
        unpadded[16..20].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(decode_frame(&unpadded), Err(ProtocolError::BadPadding));
    }

    #[test]
    fn message_type_codes_are_frozen_for_v1() {
        // These numbers are the wire contract, mirrored into TypeScript. A
        // renumbering is a protocol version bump, not a refactor.
        assert_eq!(MessageType::MetaGraphMeta.code(), 1);
        assert_eq!(MessageType::Points.code(), 2);
        assert_eq!(MessageType::Links.code(), 3);
        assert_eq!(MessageType::SessionInfo.code(), 4);
        assert_eq!(MessageType::Error.code(), 5);
        assert_eq!(MessageType::ALL.len(), 5, "a new variant must join ALL");
    }

    #[test]
    fn the_chunk_target_stays_inside_the_plans_window() {
        assert!((256 * 1024..=1024 * 1024).contains(&CHUNK_TARGET_BYTES));
        assert_eq!(chunk_bytes() % RECORD_BYTES, 0);
    }
}
