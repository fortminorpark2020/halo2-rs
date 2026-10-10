//! The relay's wire format: one frame per UDP datagram, a fixed 40-byte
//! header and, on data frames, the engine's packet exactly as it was handed
//! over (hellos and challenges carry a cookie instead). All numbers are
//! little-endian.
//!
//! | Offset | Size | Field   | Meaning |
//! |-------:|-----:|---------|---------|
//! | 0      | 4    | magic   | `H2RL` |
//! | 4      | 2    | version | [`RELAY_PROTOCOL`] |
//! | 6      | 1    | kind    | [`Kind`] |
//! | 7      | 1    | flags   | [`FLAG_SERVER`], [`FLAG_PEER`], [`FLAG_SYN`]; other bits must be 0 |
//! | 8      | 8    | room    | the match's token |
//! | 16     | 8    | src     | the sender's network id (never [`BROADCAST`]) |
//! | 24     | 8    | dst     | the receiver's network id, or [`BROADCAST`] |
//! | 32     | 4    | port    | data: the engine's port, carried unchanged; ack: the cumulative ack |
//! | 36     | 4    | seq     | sequence number; refused: the [`Refusal`] code |
//! | 40     | ..   | payload | see below |
//!
//! Payloads: data frames carry the engine's packet, at most [`MAX_PAYLOAD`]
//! bytes. A client's hello carries exactly [`HELLO_LEN`] bytes: the cookie
//! from the server's challenge (zeros before it has one), then the proof
//! of its member key (zeros without one; see the `keys` module). A
//! challenge carries the cookie, [`COOKIE_LEN`] bytes. Nothing else has a
//! payload. A client's first hello is bigger than any answer to it, so the
//! server never sends more than it was sent to an address it hasn't
//! checked.
//!
//! The payload's length is the datagram's length less the header: there is
//! no length field to disagree with it. [`decode`] checks every field and
//! returns an error for anything a well-behaved peer would never send, and
//! [`Frame::encode`] refuses to write such a frame, so what one end writes
//! the other always reads.

pub use crate::keys::{COOKIE_LEN, HELLO_LEN};
use std::fmt;

/// The first four bytes of every frame.
pub const MAGIC: [u8; 4] = *b"H2RL";
/// The relay's own protocol version, separate from h2net's `PROTOCOL`.
/// Bump it on any change to this format or to what the frames mean.
/// 2: address cookies (the challenge), member keys, the "no stream" ack,
/// and payloads up to 4 KiB.
pub const RELAY_PROTOCOL: u16 = 2;
/// The header's size in bytes.
pub const HEADER_LEN: usize = 40;
/// Where a frame's seq is, for numbering a frame already written.
const SEQ_AT: usize = 36;
/// The largest payload a data frame carries. Xbox Halo 2 sent engine
/// packets of at most 1304 bytes (0x518), and Vista's at most 1264, but
/// MCC's sizes are unknown until the launcher logs them, so this takes up
/// to 4 KiB (the research's advice). Anything over [`MTU_PAYLOAD`] goes as
/// a fragmented IP datagram, which works on most paths but is lost whole if
/// one fragment is; the client counts those sends.
pub const MAX_PAYLOAD: usize = 4096;
/// The largest payload whose frame (1440 bytes) fits a 1500-byte Ethernet
/// MTU with IPv4 or IPv6 and UDP headers, unfragmented.
pub const MTU_PAYLOAD: usize = 1400;
/// The largest frame on the wire.
pub const MAX_FRAME: usize = HEADER_LEN + MAX_PAYLOAD;
/// The destination meaning every other member of the room (unreliable data
/// and peer pings only). Never a member's own id.
pub const BROADCAST: u64 = u64::MAX;

/// Made by the relay server itself: its answer to a hello, its pong to a
/// keepalive, a refusal. The server drops any frame from a client that
/// has it, so a client can't pass itself off as the server.
pub const FLAG_SERVER: u8 = 0x01;
/// A ping or pong between two members (passed on by the server like data),
/// rather than a keepalive to the server and its answer.
pub const FLAG_PEER: u8 = 0x02;
/// On a reliable frame with no payload: "my reliable stream to you starts
/// here, at this frame's seq". On an ack: "I have no stream from you" (the
/// receiver started after the sender's stream did), for the frame `seq`;
/// the sender then starts a new one. See the client's module docs.
pub const FLAG_SYN: u8 = 0x04;
const KNOWN_FLAGS: u8 = FLAG_SERVER | FLAG_PEER | FLAG_SYN;

/// What a frame is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Kind {
    /// An engine packet sent best-effort (the engine's unreliable send).
    Unreliable = 0,
    /// An engine packet delivered once and in order (the engine's reliable
    /// send), or the start of such a stream ([`FLAG_SYN`]).
    Reliable = 1,
    /// A client joining a room (with the cookie from a challenge); from the
    /// server ([`FLAG_SERVER`]), its yes.
    Hello = 2,
    /// A client leaving its room.
    Bye = 3,
    /// A receiver's ack for a reliable frame: `seq` is the frame acked,
    /// `port` the next seq it expects (all before it arrived).
    Ack = 4,
    /// A keepalive to the server, or ([`FLAG_PEER`]) a ping to a member.
    Ping = 5,
    /// The answer to a ping, with its seq.
    Pong = 6,
    /// The server saying no; `seq` is the [`Refusal`] code.
    Refused = 7,
    /// The server's answer to a hello without a good cookie: the cookie to
    /// say hello again with, from the same address. `seq` is the hello's.
    Challenge = 8,
}

impl Kind {
    pub fn from_u8(byte: u8) -> Option<Kind> {
        Some(match byte {
            0 => Kind::Unreliable,
            1 => Kind::Reliable,
            2 => Kind::Hello,
            3 => Kind::Bye,
            4 => Kind::Ack,
            5 => Kind::Ping,
            6 => Kind::Pong,
            7 => Kind::Refused,
            8 => Kind::Challenge,
            _ => return None,
        })
    }

    /// It carries an engine packet.
    pub fn is_data(self) -> bool {
        matches!(self, Kind::Unreliable | Kind::Reliable)
    }
}

/// Why the server refused a client (a refused frame's `seq`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// The room has its most members already.
    RoomFull,
    /// The server only admits rooms it issued, and not this one.
    NoSuchRoom,
    /// The server has its most rooms already.
    TooManyRooms,
    /// The sender isn't a member of the room at that address: the server
    /// forgot it (restarted, or it was quiet too long) or its address
    /// changed (a NAT rebind). It should say hello again.
    NotMember,
    /// Too many members come from that IP address already (or it opened
    /// too many rooms lately).
    TooManyFromAddress,
    /// The room was issued by h2live and the hello didn't prove the
    /// member key for that id.
    BadKey,
    /// A code this version doesn't know.
    Other(u32),
}

impl Refusal {
    pub fn code(self) -> u32 {
        match self {
            Refusal::RoomFull => 1,
            Refusal::NoSuchRoom => 2,
            Refusal::TooManyRooms => 3,
            Refusal::NotMember => 4,
            Refusal::TooManyFromAddress => 5,
            Refusal::BadKey => 6,
            Refusal::Other(code) => code,
        }
    }

    pub fn from_code(code: u32) -> Refusal {
        match code {
            1 => Refusal::RoomFull,
            2 => Refusal::NoSuchRoom,
            3 => Refusal::TooManyRooms,
            4 => Refusal::NotMember,
            5 => Refusal::TooManyFromAddress,
            6 => Refusal::BadKey,
            _ => Refusal::Other(code),
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Refusal::RoomFull => write!(f, "the room is full"),
            Refusal::NoSuchRoom => write!(f, "no such room"),
            Refusal::TooManyRooms => write!(f, "the relay has too many rooms"),
            Refusal::NotMember => write!(f, "not a member of the room"),
            Refusal::TooManyFromAddress => write!(f, "too many from one address"),
            Refusal::BadKey => write!(f, "wrong member key"),
            Refusal::Other(code) => write!(f, "refused ({code})"),
        }
    }
}

/// One frame, its payload borrowed from the datagram (or the caller).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    pub kind: Kind,
    pub flags: u8,
    pub room: u64,
    pub src: u64,
    pub dst: u64,
    pub port: u32,
    pub seq: u32,
    pub payload: &'a [u8],
}

/// What's wrong with a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// Shorter than a header (the length it had).
    Truncated(usize),
    BadMagic,
    /// Another relay protocol version.
    BadVersion(u16),
    BadKind(u8),
    /// Flags unknown, or not allowed on this kind.
    BadFlags(u8),
    /// The source is [`BROADCAST`].
    BadSource,
    /// [`BROADCAST`] on a kind that can't be broadcast.
    BadDestination,
    /// A data payload over [`MAX_PAYLOAD`] (its length).
    PayloadTooLarge(usize),
    /// A payload on a frame that carries none, or the wrong length for a
    /// hello's or a challenge's.
    UnexpectedPayload,
    /// `encode`'s buffer is too small (the length the frame needs).
    BufferTooSmall(usize),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FrameError::Truncated(n) => write!(f, "{n} bytes, shorter than a header"),
            FrameError::BadMagic => write!(f, "not a relay frame"),
            FrameError::BadVersion(v) => {
                write!(f, "relay protocol {v}, not {RELAY_PROTOCOL}")
            }
            FrameError::BadKind(k) => write!(f, "unknown kind {k}"),
            FrameError::BadFlags(flags) => write!(f, "flags {flags:#04x} not allowed"),
            FrameError::BadSource => write!(f, "broadcast as the source"),
            FrameError::BadDestination => write!(f, "broadcast not allowed on this kind"),
            FrameError::PayloadTooLarge(n) => {
                write!(f, "payload of {n} bytes, over {MAX_PAYLOAD}")
            }
            FrameError::UnexpectedPayload => write!(f, "payload on a frame that has none"),
            FrameError::BufferTooSmall(n) => write!(f, "buffer too small for {n} bytes"),
        }
    }
}

impl std::error::Error for FrameError {}

impl<'a> Frame<'a> {
    /// A frame of `kind` with no flags, payload, port or seq.
    pub fn new(kind: Kind, room: u64, src: u64, dst: u64) -> Frame<'static> {
        Frame {
            kind,
            flags: 0,
            room,
            src,
            dst,
            port: 0,
            seq: 0,
            payload: &[],
        }
    }

    pub fn with_flags(self, flags: u8) -> Frame<'a> {
        Frame { flags, ..self }
    }

    pub fn with_port(self, port: u32) -> Frame<'a> {
        Frame { port, ..self }
    }

    pub fn with_seq(self, seq: u32) -> Frame<'a> {
        Frame { seq, ..self }
    }

    pub fn with_payload<'b>(self, payload: &'b [u8]) -> Frame<'b> {
        Frame {
            kind: self.kind,
            flags: self.flags,
            room: self.room,
            src: self.src,
            dst: self.dst,
            port: self.port,
            seq: self.seq,
            payload,
        }
    }

    /// Its length on the wire.
    pub fn len(&self) -> usize {
        HEADER_LEN + self.payload.len()
    }

    /// Never: a frame always has a header.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Whether a well-behaved end could send it.
    pub fn validate(&self) -> Result<(), FrameError> {
        let flags = self.flags;
        if flags & !KNOWN_FLAGS != 0 {
            return Err(FrameError::BadFlags(flags));
        }
        let server = flags & FLAG_SERVER != 0;
        let peer = flags & FLAG_PEER != 0;
        let syn = flags & FLAG_SYN != 0;
        let allowed = match self.kind {
            Kind::Unreliable | Kind::Bye => flags == 0,
            Kind::Ack => flags == 0 || flags == FLAG_SYN,
            Kind::Reliable => !server && !peer,
            Kind::Hello => !peer && !syn,
            Kind::Challenge => flags == FLAG_SERVER,
            Kind::Ping => !server && !syn,
            // From the server, or from a member: one or the other.
            Kind::Pong => server != peer && !syn,
            Kind::Refused => server && !peer && !syn,
        };
        if !allowed {
            return Err(FrameError::BadFlags(flags));
        }
        if self.src == BROADCAST {
            return Err(FrameError::BadSource);
        }
        let broadcast = match self.kind {
            Kind::Unreliable => true,
            Kind::Ping => peer,
            _ => false,
        };
        if self.dst == BROADCAST && !broadcast {
            return Err(FrameError::BadDestination);
        }
        let len = self.payload.len();
        let fits = match self.kind {
            Kind::Unreliable => true,
            Kind::Reliable => !syn || len == 0,
            Kind::Hello if server => len == 0,
            Kind::Hello => len == HELLO_LEN,
            Kind::Challenge => len == COOKIE_LEN,
            _ => len == 0,
        };
        if !fits {
            return Err(FrameError::UnexpectedPayload);
        }
        if len > MAX_PAYLOAD {
            return Err(FrameError::PayloadTooLarge(len));
        }
        Ok(())
    }

    /// Write it to the start of `out`: its length.
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, FrameError> {
        self.validate()?;
        let len = self.len();
        let Some(out) = out.get_mut(..len) else {
            return Err(FrameError::BufferTooSmall(len));
        };
        out[0..4].copy_from_slice(&MAGIC);
        out[4..6].copy_from_slice(&RELAY_PROTOCOL.to_le_bytes());
        out[6] = self.kind as u8;
        out[7] = self.flags;
        out[8..16].copy_from_slice(&self.room.to_le_bytes());
        out[16..24].copy_from_slice(&self.src.to_le_bytes());
        out[24..32].copy_from_slice(&self.dst.to_le_bytes());
        out[32..36].copy_from_slice(&self.port.to_le_bytes());
        out[36..40].copy_from_slice(&self.seq.to_le_bytes());
        out[HEADER_LEN..].copy_from_slice(self.payload);
        Ok(len)
    }

    /// It encoded, in a buffer of its own.
    pub fn to_vec(&self) -> Result<Vec<u8>, FrameError> {
        let mut out = vec![0; self.len()];
        self.encode(&mut out)?;
        Ok(out)
    }
}

/// Read a frame from one whole datagram.
pub fn decode(bytes: &[u8]) -> Result<Frame<'_>, FrameError> {
    let Some((header, payload)) = bytes.split_first_chunk::<HEADER_LEN>() else {
        return Err(FrameError::Truncated(bytes.len()));
    };
    if header[0..4] != MAGIC {
        return Err(FrameError::BadMagic);
    }
    let u16_at = |at: usize| u16::from_le_bytes([header[at], header[at + 1]]);
    let u32_at = |at: usize| {
        let mut b = [0; 4];
        b.copy_from_slice(&header[at..at + 4]);
        u32::from_le_bytes(b)
    };
    let u64_at = |at: usize| {
        let mut b = [0; 8];
        b.copy_from_slice(&header[at..at + 8]);
        u64::from_le_bytes(b)
    };
    let version = u16_at(4);
    if version != RELAY_PROTOCOL {
        return Err(FrameError::BadVersion(version));
    }
    let kind = Kind::from_u8(header[6]).ok_or(FrameError::BadKind(header[6]))?;
    let frame = Frame {
        kind,
        flags: header[7],
        room: u64_at(8),
        src: u64_at(16),
        dst: u64_at(24),
        port: u32_at(32),
        seq: u32_at(36),
        payload,
    };
    frame.validate()?;
    Ok(frame)
}

/// Give the frame written in `bytes` the seq `seq`.
pub(crate) fn set_seq(bytes: &mut [u8], seq: u32) {
    if let Some(at) = bytes.get_mut(SEQ_AT..SEQ_AT + 4) {
        at.copy_from_slice(&seq.to_le_bytes());
    }
}

/// How far `seq` is after `base` in a wrapping 32-bit sequence space
/// (negative when before).
pub fn seq_after(seq: u32, base: u32) -> i32 {
    seq.wrapping_sub(base) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: u64 = 0x0123_4567_89ab_cdef;

    /// One well-formed frame of each shape there is.
    fn samples() -> Vec<Vec<u8>> {
        let big = vec![0xa5; MAX_PAYLOAD];
        let hello = [0x3c; HELLO_LEN];
        let frames = [
            Frame::new(Kind::Unreliable, ROOM, 1, 2)
                .with_port(1000)
                .with_seq(7)
                .with_payload(b"engine bytes"),
            Frame::new(Kind::Unreliable, ROOM, 1, BROADCAST).with_payload(&big),
            Frame::new(Kind::Unreliable, ROOM, 1, 2),
            Frame::new(Kind::Reliable, ROOM, 3, 4)
                .with_port(u32::MAX)
                .with_seq(u32::MAX)
                .with_payload(&big[..1304]),
            Frame::new(Kind::Reliable, ROOM, 3, 4)
                .with_flags(FLAG_SYN)
                .with_seq(99),
            Frame::new(Kind::Hello, ROOM, 5, 0)
                .with_seq(42)
                .with_payload(&hello),
            Frame::new(Kind::Hello, ROOM, 0, 5)
                .with_flags(FLAG_SERVER)
                .with_seq(42)
                .with_port(3),
            Frame::new(Kind::Challenge, ROOM, 0, 5)
                .with_flags(FLAG_SERVER)
                .with_seq(42)
                .with_payload(&hello[..COOKIE_LEN]),
            Frame::new(Kind::Bye, ROOM, 5, 0),
            Frame::new(Kind::Ack, ROOM, 4, 3)
                .with_seq(100)
                .with_port(101),
            Frame::new(Kind::Ack, ROOM, 4, 3)
                .with_flags(FLAG_SYN)
                .with_seq(100),
            Frame::new(Kind::Ping, ROOM, 5, 0).with_seq(1),
            Frame::new(Kind::Ping, ROOM, 5, BROADCAST).with_flags(FLAG_PEER),
            Frame::new(Kind::Pong, ROOM, 0, 5).with_flags(FLAG_SERVER),
            Frame::new(Kind::Pong, ROOM, 6, 5).with_flags(FLAG_PEER),
            Frame::new(Kind::Refused, ROOM, 0, 5)
                .with_flags(FLAG_SERVER)
                .with_seq(Refusal::RoomFull.code()),
        ];
        frames.iter().map(|f| f.to_vec().unwrap()).collect()
    }

    #[test]
    fn frames_round_trip() {
        let payload: Vec<u8> = (0..=255).cycle().take(1304).collect();
        let frame = Frame::new(Kind::Reliable, u64::MAX - 1, 0x1122_3344_5566_7788, 9)
            .with_port(1001)
            .with_seq(0xdead_beef)
            .with_payload(&payload);
        let bytes = frame.to_vec().unwrap();
        assert_eq!(bytes.len(), HEADER_LEN + 1304);
        assert_eq!(&bytes[..4], b"H2RL");
        assert_eq!(bytes[4..6], RELAY_PROTOCOL.to_le_bytes());
        // Little-endian, at the documented offsets.
        assert_eq!(bytes[16..24], 0x1122_3344_5566_7788u64.to_le_bytes());
        assert_eq!(bytes[32..36], 1001u32.to_le_bytes());
        assert_eq!(decode(&bytes), Ok(frame));
        for bytes in samples() {
            let frame = decode(&bytes).unwrap();
            assert_eq!(frame.to_vec().unwrap(), bytes);
        }
        for code in 0..8 {
            assert_eq!(Refusal::from_code(code).code(), code);
        }
        // Numbering a frame already written.
        let mut bytes = samples().remove(0);
        set_seq(&mut bytes, 0x0102_0304);
        assert_eq!(decode(&bytes).unwrap().seq, 0x0102_0304);
    }

    #[test]
    fn malformed_frames_are_refused() {
        let good = samples().remove(0);
        // Every length short of a header.
        for n in 0..HEADER_LEN {
            assert_eq!(decode(&good[..n]), Err(FrameError::Truncated(n)));
        }
        let with = |at: usize, byte: u8| {
            let mut b = good.clone();
            b[at] = byte;
            b
        };
        assert_eq!(decode(&with(0, b'X')), Err(FrameError::BadMagic));
        assert_eq!(decode(&with(4, 1)), Err(FrameError::BadVersion(1)));
        assert_eq!(decode(&with(5, 1)), Err(FrameError::BadVersion(258)));
        assert_eq!(decode(&with(6, 9)), Err(FrameError::BadKind(9)));
        assert_eq!(decode(&with(7, 0x80)), Err(FrameError::BadFlags(0x80)));
        // Flags that don't belong on the kind: a client can't be the server.
        assert_eq!(
            decode(&with(7, FLAG_SERVER)),
            Err(FrameError::BadFlags(FLAG_SERVER))
        );
        let mut syn_with_payload = good.clone();
        syn_with_payload[6] = Kind::Reliable as u8;
        syn_with_payload[7] = FLAG_SYN;
        assert_eq!(
            decode(&syn_with_payload),
            Err(FrameError::UnexpectedPayload)
        );
        // Payloads only on data, and hellos' and challenges' just so long.
        let mut hello = good.clone();
        hello[6] = Kind::Hello as u8;
        assert_eq!(decode(&hello), Err(FrameError::UnexpectedPayload));
        let mut ack = good[..HEADER_LEN].to_vec();
        ack[6] = Kind::Ack as u8;
        assert!(decode(&ack).is_ok());
        ack.push(0);
        assert_eq!(decode(&ack), Err(FrameError::UnexpectedPayload));
        let short_hello = Frame::new(Kind::Hello, ROOM, 1, 0).with_payload(&[0; COOKIE_LEN]);
        assert_eq!(short_hello.to_vec(), Err(FrameError::UnexpectedPayload));
        assert_eq!(
            Frame::new(Kind::Hello, ROOM, 1, 0).to_vec(),
            Err(FrameError::UnexpectedPayload)
        );
        let challenge = Frame::new(Kind::Challenge, ROOM, 0, 1).with_flags(FLAG_SERVER);
        assert_eq!(challenge.to_vec(), Err(FrameError::UnexpectedPayload));
        // Only the server challenges; only an ack may say "no stream".
        let from_client = Frame::new(Kind::Challenge, ROOM, 0, 1).with_payload(&[0; COOKIE_LEN]);
        assert_eq!(from_client.to_vec(), Err(FrameError::BadFlags(0)));
        let syn_data = Frame::new(Kind::Unreliable, ROOM, 1, 2).with_flags(FLAG_SYN);
        assert_eq!(syn_data.to_vec(), Err(FrameError::BadFlags(FLAG_SYN)));
        // Broadcast as the source, or where it can't go.
        let mut source = good.clone();
        source[16..24].copy_from_slice(&BROADCAST.to_le_bytes());
        assert_eq!(decode(&source), Err(FrameError::BadSource));
        let mut reliable = good.clone();
        reliable[6] = Kind::Reliable as u8;
        reliable[24..32].copy_from_slice(&BROADCAST.to_le_bytes());
        assert_eq!(decode(&reliable), Err(FrameError::BadDestination));
        // Oversize.
        let mut big = good[..HEADER_LEN].to_vec();
        big.resize(HEADER_LEN + MAX_PAYLOAD + 1, 0);
        assert_eq!(
            decode(&big),
            Err(FrameError::PayloadTooLarge(MAX_PAYLOAD + 1))
        );
        let huge = vec![0; 1 << 16];
        let frame = Frame::new(Kind::Unreliable, ROOM, 1, 2).with_payload(&huge);
        assert_eq!(frame.to_vec(), Err(FrameError::PayloadTooLarge(1 << 16)));
        // Encoding checks the same, and the buffer.
        let bad = Frame::new(Kind::Reliable, ROOM, 1, BROADCAST);
        assert_eq!(bad.to_vec(), Err(FrameError::BadDestination));
        let mut small = [0; HEADER_LEN + 3];
        let frame = Frame::new(Kind::Unreliable, ROOM, 1, 2).with_payload(b"four");
        assert_eq!(
            frame.encode(&mut small),
            Err(FrameError::BufferTooSmall(HEADER_LEN + 4))
        );
    }

    /// A small random number source for the fuzz loops.
    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    #[test]
    fn hostile_bytes_never_panic() {
        let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
        let mut buf = vec![0u8; 2 * MAX_FRAME];
        // Random bytes, with the right magic and version half the time so
        // the field checks get exercised too.
        for i in 0..20_000 {
            let len = (rng.next() % (MAX_FRAME as u64 + 200)) as usize;
            for b in &mut buf[..len] {
                *b = rng.next() as u8;
            }
            if i % 2 == 0 && len >= 6 {
                buf[..4].copy_from_slice(&MAGIC);
                buf[4..6].copy_from_slice(&RELAY_PROTOCOL.to_le_bytes());
                buf[6] %= 10;
                buf[7] &= 0x0f;
            }
            if let Ok(frame) = decode(&buf[..len]) {
                // What it reads, it writes back the same.
                assert_eq!(frame.to_vec().unwrap(), &buf[..len]);
            }
        }
        // Good frames with bits flipped and ends cut off.
        let samples = samples();
        for i in 0..20_000 {
            let mut bytes = samples[i % samples.len()].clone();
            for _ in 0..1 + rng.next() % 3 {
                let at = (rng.next() as usize) % bytes.len();
                bytes[at] ^= 1 << (rng.next() % 8);
            }
            let cut = (rng.next() as usize) % (bytes.len() + 1);
            for bytes in [&bytes[..], &bytes[..cut]] {
                if let Ok(frame) = decode(bytes) {
                    assert_eq!(frame.to_vec().unwrap(), bytes);
                }
            }
        }
    }

    #[test]
    fn sequence_numbers_wrap() {
        assert_eq!(seq_after(5, 3), 2);
        assert_eq!(seq_after(3, 5), -2);
        assert_eq!(seq_after(1, u32::MAX), 2);
        assert_eq!(seq_after(u32::MAX, 1), -2);
    }
}
