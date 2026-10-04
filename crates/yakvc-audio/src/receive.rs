use std::time::{Duration, Instant};

use crate::MonoFrame;

/// One received voice packet.
#[derive(Debug, Clone, Copy)]
pub struct Packet<'a> {
    pub seq: u32,
    /// Capture time of the first sample, in 48 kHz samples.
    pub ts: u32,
    pub end_of_talk: bool,
    pub payload: &'a [u8],
}

#[derive(Debug, Clone, PartialEq)]
pub struct JitterConfig {
    /// Playout delay the buffer aims for.
    pub target: Duration,
    pub min: Duration,
    pub max: Duration,
}

impl Default for JitterConfig {
    fn default() -> Self {
        JitterConfig {
            target: Duration::from_millis(40),
            min: Duration::from_millis(20),
            max: Duration::from_millis(200),
        }
    }
}

/// One remote speaker's receive pipeline: adaptive jitter buffer plus Opus
/// decoder with FEC and packet-loss concealment.
#[derive(Debug)]
pub struct ReceiveStream {
    _p: (),
}

/// What [`ReceiveStream::pull`] produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pulled {
    /// Decoded or concealed speech.
    Voice,
    /// Nothing to play; the frame was zeroed.
    Silence,
}

/// Counters since the stream was created.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StreamStats {
    pub received: u64,
    /// Arrived after their playout time and were dropped.
    pub late: u64,
    /// Missing frames rebuilt from the next packet's FEC data.
    pub fec_recovered: u64,
    /// Missing frames filled by packet-loss concealment.
    pub concealed: u64,
    /// Current playout delay.
    pub playout_delay: Duration,
}

impl ReceiveStream {
    pub fn new(config: JitterConfig) -> Self {
        let _ = config;
        todo!()
    }

    /// Queues a packet that arrived at `now`.
    pub fn push(&mut self, packet: Packet<'_>, now: Instant) {
        let _ = (packet, now);
        todo!()
    }

    /// Produces the next 20 ms of audio for playout at `now`.
    pub fn pull(&mut self, out: &mut MonoFrame, now: Instant) -> Pulled {
        let _ = (out, now);
        todo!()
    }

    pub fn stats(&self) -> StreamStats {
        todo!()
    }
}
