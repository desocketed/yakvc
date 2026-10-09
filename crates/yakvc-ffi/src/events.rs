//! Encodes engine events as the TLV records described in the crate docs.

use std::time::UNIX_EPOCH;

use yakvc_client::{Event, Events, GroupNoticeKind, PeerState, RendezvousState};

use crate::guard::FfiError;
use crate::*;

/// Size of the `u16 type | u16 len` record header.
const HEADER_LEN: usize = 4;

/// The engine's event queue plus at most one record that did not fit in the
/// caller's last buffer.
#[derive(Debug)]
pub(crate) struct EventQueue {
    events: Events,
    held: Option<Vec<u8>>,
}

impl EventQueue {
    pub fn new(events: Events) -> Self {
        EventQueue { events, held: None }
    }

    /// Fills `buf` with whole records and returns the bytes written.
    pub fn drain_into(&mut self, buf: &mut [u8]) -> Result<usize, FfiError> {
        write_records(&mut self.held, || self.events.try_next(), buf)
    }
}

/// Writes records for `next()`'s events into `buf` until it runs dry or the
/// next record doesn't fit. A record that doesn't fit is kept in `held` and
/// written first next time, so records are never split or lost.
fn write_records(
    held: &mut Option<Vec<u8>>,
    mut next: impl FnMut() -> Option<Event>,
    buf: &mut [u8],
) -> Result<usize, FfiError> {
    let mut written = 0;
    loop {
        let record = match held.take() {
            Some(record) => record,
            None => match next() {
                Some(event) => encode(&event),
                None => return Ok(written),
            },
        };
        let end = written + record.len();
        if end > buf.len() {
            let too_small = written == 0;
            let needed = record.len();
            *held = Some(record);
            if too_small {
                // Java would otherwise poll forever without making progress.
                return Err(FfiError::invalid(format!(
                    "buffer of {} bytes is too small for the next event ({needed} bytes)",
                    buf.len()
                )));
            }
            return Ok(written);
        }
        buf[written..end].copy_from_slice(&record);
        written = end;
    }
}

/// Encodes one event as a `u16 type | u16 len | payload` record.
pub(crate) fn encode(event: &Event) -> Vec<u8> {
    let mut payload = Vec::new();
    let kind = match event {
        Event::JoinRequest { id, server_id } => {
            payload.extend(id.0.to_le_bytes());
            payload.extend(server_id.as_bytes());
            YAKVC_EVENT_JOIN_REQUEST
        }
        Event::Rendezvous(state) => {
            let (code, value, verified) = match state {
                RendezvousState::Connecting => (YAKVC_RDV_CONNECTING, 0, false),
                RendezvousState::Authenticating => (YAKVC_RDV_AUTHENTICATING, 0, false),
                RendezvousState::Registered {
                    expires_at,
                    verified,
                } => {
                    let secs = expires_at
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    (YAKVC_RDV_REGISTERED, secs, *verified)
                }
                RendezvousState::RetryingIn(delay) => {
                    let millis = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
                    (YAKVC_RDV_RETRYING, millis, false)
                }
                RendezvousState::Disconnected => (YAKVC_RDV_DISCONNECTED, 0, false),
            };
            payload.push(code);
            payload.extend(value.to_le_bytes());
            payload.push(u8::from(verified));
            YAKVC_EVENT_RENDEZVOUS_STATE
        }
        Event::Peer {
            uuid,
            state,
            verified,
        } => {
            payload.extend(uuid.as_bytes());
            payload.push(match state {
                PeerState::Connecting => YAKVC_PEER_CONNECTING,
                PeerState::Direct => YAKVC_PEER_DIRECT,
                PeerState::Relayed => YAKVC_PEER_RELAYED,
                PeerState::RelayFull => YAKVC_PEER_RELAY_FULL,
                PeerState::Failed => YAKVC_PEER_FAILED,
                PeerState::Gone => YAKVC_PEER_GONE,
            });
            payload.push(u8::from(*verified));
            YAKVC_EVENT_PEER_STATE
        }
        Event::Talking { uuid, talking } => {
            payload.extend(uuid.as_bytes());
            payload.push(u8::from(*talking));
            YAKVC_EVENT_TALKING
        }
        Event::Invite { from } => {
            payload.extend(from.as_bytes());
            YAKVC_EVENT_INVITE
        }
        Event::GroupNotice { kind, uuid, by } => {
            payload.push(match kind {
                GroupNoticeKind::Joined => YAKVC_GROUP_JOINED,
                GroupNoticeKind::Left => YAKVC_GROUP_LEFT,
                GroupNoticeKind::KickedUs => YAKVC_GROUP_KICKED_US,
                GroupNoticeKind::MadePublic => YAKVC_GROUP_MADE_PUBLIC,
                GroupNoticeKind::MadePrivate => YAKVC_GROUP_MADE_PRIVATE,
                GroupNoticeKind::LabelChanged => YAKVC_GROUP_LABEL_CHANGED,
            });
            payload.extend(uuid.as_bytes());
            payload.extend(by.as_bytes());
            YAKVC_EVENT_GROUP_NOTICE
        }
        Event::KickVote {
            target,
            by,
            yes,
            votes,
            needed,
        } => {
            payload.extend(target.as_bytes());
            payload.extend(by.as_bytes());
            payload.push(u8::from(*yes));
            payload.extend(votes.to_le_bytes());
            payload.extend(needed.to_le_bytes());
            YAKVC_EVENT_KICK_VOTE
        }
        Event::MicLevel(db) => {
            payload.extend(db.to_le_bytes());
            YAKVC_EVENT_MIC_LEVEL
        }
        Event::Error(message) => {
            // The length field is a u16, so cut very long messages short at a
            // character boundary.
            let end = message.floor_char_boundary(usize::from(u16::MAX));
            payload.extend(&message.as_bytes()[..end]);
            YAKVC_EVENT_ERROR
        }
    };
    let len = u16::try_from(payload.len()).expect("event payloads fit in a u16");

    let mut record = Vec::with_capacity(HEADER_LEN + payload.len());
    record.extend(kind.to_le_bytes());
    record.extend(len.to_le_bytes());
    record.extend(payload);
    record
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::Duration;

    use yakvc_client::{JoinId, Uuid};

    use super::*;

    const UUID: Uuid = Uuid::from_u128(0x0011_2233_4455_6677_8899_aabb_ccdd_eeff);
    const UUID_BYTES: [u8; 16] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];

    fn record(kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut bytes = kind.to_le_bytes().to_vec();
        bytes.extend((payload.len() as u16).to_le_bytes());
        bytes.extend(payload);
        bytes
    }

    #[test]
    fn encodes_join_request() {
        let event = Event::JoinRequest {
            id: JoinId(0x0102_0304),
            server_id: "-7f".to_owned(),
        };
        let payload = [4, 3, 2, 1, b'-', b'7', b'f'];
        assert_eq!(encode(&event), record(YAKVC_EVENT_JOIN_REQUEST, &payload));
    }

    #[test]
    fn encodes_rendezvous_states() {
        let registered = Event::Rendezvous(RendezvousState::Registered {
            expires_at: UNIX_EPOCH + Duration::from_secs(1_800_000_000),
            verified: true,
        });
        let mut payload = vec![YAKVC_RDV_REGISTERED];
        payload.extend(1_800_000_000u64.to_le_bytes());
        payload.push(1);
        assert_eq!(
            encode(&registered),
            record(YAKVC_EVENT_RENDEZVOUS_STATE, &payload)
        );

        let retrying = Event::Rendezvous(RendezvousState::RetryingIn(Duration::from_secs(10)));
        let mut payload = vec![YAKVC_RDV_RETRYING];
        payload.extend(10_000u64.to_le_bytes());
        payload.push(0);
        assert_eq!(
            encode(&retrying),
            record(YAKVC_EVENT_RENDEZVOUS_STATE, &payload)
        );

        let connecting = Event::Rendezvous(RendezvousState::Connecting);
        let mut payload = vec![YAKVC_RDV_CONNECTING];
        payload.extend(0u64.to_le_bytes());
        payload.push(0);
        assert_eq!(
            encode(&connecting),
            record(YAKVC_EVENT_RENDEZVOUS_STATE, &payload)
        );
    }

    #[test]
    fn encodes_peer_state() {
        let event = Event::Peer {
            uuid: UUID,
            state: PeerState::RelayFull,
            verified: true,
        };
        let mut payload = UUID_BYTES.to_vec();
        payload.push(YAKVC_PEER_RELAY_FULL);
        payload.push(1);
        assert_eq!(encode(&event), record(YAKVC_EVENT_PEER_STATE, &payload));
    }

    #[test]
    fn encodes_talking() {
        let event = Event::Talking {
            uuid: UUID,
            talking: true,
        };
        let mut payload = UUID_BYTES.to_vec();
        payload.push(1);
        assert_eq!(encode(&event), record(YAKVC_EVENT_TALKING, &payload));
    }

    #[test]
    fn encodes_group_events() {
        let invite = Event::Invite { from: UUID };
        assert_eq!(encode(&invite), record(YAKVC_EVENT_INVITE, &UUID_BYTES));

        let notice = Event::GroupNotice {
            kind: GroupNoticeKind::MadePublic,
            uuid: Uuid::nil(),
            by: UUID,
        };
        let mut payload = vec![YAKVC_GROUP_MADE_PUBLIC];
        payload.extend([0; 16]);
        payload.extend(UUID_BYTES);
        assert_eq!(encode(&notice), record(YAKVC_EVENT_GROUP_NOTICE, &payload));

        let vote = Event::KickVote {
            target: UUID,
            by: Uuid::nil(),
            yes: true,
            votes: 2,
            needed: 3,
        };
        let mut payload = UUID_BYTES.to_vec();
        payload.extend([0; 16]);
        payload.push(1);
        payload.extend([2, 0, 0, 0, 3, 0, 0, 0]);
        assert_eq!(encode(&vote), record(YAKVC_EVENT_KICK_VOTE, &payload));
    }

    #[test]
    fn encodes_mic_level_and_error() {
        let level = encode(&Event::MicLevel(-12.5));
        assert_eq!(
            level,
            record(YAKVC_EVENT_MIC_LEVEL, &(-12.5f32).to_le_bytes())
        );

        let error = encode(&Event::Error("no mic".to_owned()));
        assert_eq!(error, record(YAKVC_EVENT_ERROR, b"no mic"));
    }

    #[test]
    fn truncates_long_errors_at_a_char_boundary() {
        // 'é' is two bytes, so u16::MAX bytes would end mid-character.
        let message = "é".repeat(40_000);
        let encoded = encode(&Event::Error(message));
        let len = u16::from_le_bytes([encoded[2], encoded[3]]);
        assert_eq!(len, u16::MAX - 1);
        assert!(std::str::from_utf8(&encoded[HEADER_LEN..]).is_ok());
    }

    fn queue(events: &[Event]) -> VecDeque<Event> {
        events.iter().cloned().collect()
    }

    #[test]
    fn writes_whole_records_and_keeps_the_rest_queued() {
        let events = [
            Event::MicLevel(-1.0),
            Event::MicLevel(-2.0),
            Event::MicLevel(-3.0),
        ];
        let mut pending = queue(&events);
        let mut held = None;
        // Room for two 8-byte records and half of the third.
        let mut buf = [0u8; 20];

        let written = write_records(&mut held, || pending.pop_front(), &mut buf).unwrap();
        assert_eq!(written, 16);
        assert_eq!(buf[..8], encode(&events[0]));
        assert_eq!(buf[8..16], encode(&events[1]));

        let written = write_records(&mut held, || pending.pop_front(), &mut buf).unwrap();
        assert_eq!(written, 8);
        assert_eq!(buf[..8], encode(&events[2]));

        let written = write_records(&mut held, || pending.pop_front(), &mut buf).unwrap();
        assert_eq!(written, 0);
    }

    #[test]
    fn a_buffer_too_small_for_one_record_is_an_error_and_loses_nothing() {
        let event = Event::Error("a long-ish message".to_owned());
        let mut pending = queue(std::slice::from_ref(&event));
        let mut held = None;

        let mut small = [0u8; 8];
        let err = write_records(&mut held, || pending.pop_front(), &mut small).unwrap_err();
        assert_eq!(err.code, YAKVC_ERR_INVALID_ARGUMENT);

        let mut big = [0u8; 64];
        let written = write_records(&mut held, || pending.pop_front(), &mut big).unwrap();
        assert_eq!(big[..written], encode(&event));
    }
}
