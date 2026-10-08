//! Wire format of the voice protocol ([`ProtocolId::VOICE`](crate::ProtocolId::VOICE)).

use serde::{Deserialize, Serialize};

use crate::group::GroupAnnounce;

/// 2 added group voice chat.
pub const VERSION: u16 = 2;

/// Messages on the voice control stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VoiceMsg {
    /// Whether the sender wants our audio (false when it deafened or muted us).
    ReceiveState { wants_audio: bool },
    /// The group the sender is in, or `None` once it leaves. Sent when the
    /// link opens and on every change.
    Group(Option<GroupAnnounce>),
}

/// Header at the start of each voice datagram, after the peer layer's
/// protocol byte: `flags: u8 | seq: u32 | ts: u32`, big-endian, then the
/// Opus payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceHeader {
    /// Increments by one per frame sent.
    pub seq: u32,
    /// Capture time of the frame's first sample, in 48 kHz samples.
    pub ts: u32,
    /// Last frame of a talk spurt.
    pub end_of_talk: bool,
    /// Sent to the group: played unpositioned, and only from group mates.
    pub group: bool,
}

impl VoiceHeader {
    pub const LEN: usize = 9;

    const END_OF_TALK: u8 = 0x01;
    const GROUP: u8 = 0x02;

    pub fn to_bytes(&self) -> [u8; Self::LEN] {
        let mut out = [0; Self::LEN];
        if self.end_of_talk {
            out[0] |= Self::END_OF_TALK;
        }
        if self.group {
            out[0] |= Self::GROUP;
        }
        out[1..5].copy_from_slice(&self.seq.to_be_bytes());
        out[5..9].copy_from_slice(&self.ts.to_be_bytes());
        out
    }

    /// Splits a datagram into header and Opus payload. `None` if too short.
    pub fn parse(datagram: &[u8]) -> Option<(VoiceHeader, &[u8])> {
        let (header, payload) = datagram.split_first_chunk::<{ Self::LEN }>()?;
        let header = VoiceHeader {
            end_of_talk: header[0] & Self::END_OF_TALK != 0,
            group: header[0] & Self::GROUP != 0,
            seq: u32::from_be_bytes(header[1..5].try_into().unwrap()),
            ts: u32::from_be_bytes(header[5..9].try_into().unwrap()),
        };
        Some((header, payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trips() {
        let header = VoiceHeader {
            seq: 7,
            ts: 960 * 7,
            end_of_talk: true,
            group: true,
        };
        let mut datagram = header.to_bytes().to_vec();
        datagram.extend_from_slice(b"opus");
        assert_eq!(VoiceHeader::parse(&datagram), Some((header, &b"opus"[..])));
        assert_eq!(VoiceHeader::parse(&datagram[..8]), None);
    }
}
