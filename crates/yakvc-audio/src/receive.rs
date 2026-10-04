use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crate::{FRAME_DURATION, FRAME_SAMPLES, MonoFrame, SAMPLE_RATE};

const FRAME_MICROS: i64 = FRAME_DURATION.as_micros() as i64;

/// Packets whose timing feeds the jitter estimate: about two seconds of
/// speech.
const JITTER_WINDOW: usize = 100;

/// Longest run of missing frames that is concealed. A longer gap means the
/// talk spurt ended without its `end_of_talk` packet.
const MAX_CONCEALED_RUN: u32 = 5;

/// A packet this far from the expected timing means the sender restarted
/// its clock, so the stream starts over.
const RESYNC_MICROS: i64 = 2_000_000;

/// Bound on queued packets, so a misbehaving peer cannot grow the queue.
const MAX_QUEUED: usize = 100;

/// One received voice packet.
#[derive(Debug, Clone, Copy)]
pub struct Packet<'a> {
    /// Increments by one per packet sent. Frames that DTX skips get no
    /// number, so a gap in `seq` means loss and a gap in `ts` alone does not.
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
///
/// Each frame has a slot: its `ts` counted in frames. Slot `n` plays at
/// `base + n * 20 ms + delay`, where `base` is the smallest recent transit
/// time (arrival minus slot time) and `delay` covers the spread of transit
/// times above it, within the configured bounds. Timing on `ts` rather than
/// arrival order is what lets the buffer reorder packets and keep pauses
/// between talk spurts.
#[derive(Debug)]
pub struct ReceiveStream {
    config: JitterConfig,
    decoder: opus::Decoder,
    /// Time zero for the microsecond times below. Set by the first packet.
    epoch: Option<Instant>,
    /// The newest packet's `ts` and slot, to number frames as `ts` wraps.
    newest_ts: u32,
    newest_slot: i64,
    /// Recent transit times in microseconds.
    transits: VecDeque<i64>,
    /// Packets waiting for playout, by slot.
    queue: BTreeMap<i64, QueuedPacket>,
    /// In a talk spurt, playing one slot per pull.
    playing: bool,
    /// The slot the next pull plays. Packets for earlier slots are late.
    next_slot: i64,
    /// `seq` of the last packet played, to tell loss from DTX.
    last_seq: u32,
    gap: Gap,
    delay: Duration,
    stats: StreamStats,
}

#[derive(Debug)]
struct QueuedPacket {
    seq: u32,
    end_of_talk: bool,
    payload: Vec<u8>,
}

/// Frames missing since the last packet played, and how they were filled.
///
/// Whether a missing frame was lost or skipped by the sender's DTX is only
/// known once a later packet shows (by its `seq`) how many packets went
/// missing, so the stats count the gap when it closes.
#[derive(Debug, Default)]
struct Gap {
    frames: u32,
    fec_recovered: u64,
    concealed: u64,
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
        let decoder = opus::Decoder::new(SAMPLE_RATE, opus::Channels::Mono)
            .expect("a mono 48 kHz Opus decoder is always valid");
        let delay = config.target.clamp(config.min, config.max);
        ReceiveStream {
            config,
            decoder,
            epoch: None,
            newest_ts: 0,
            newest_slot: 0,
            transits: VecDeque::with_capacity(JITTER_WINDOW),
            queue: BTreeMap::new(),
            playing: false,
            next_slot: i64::MIN,
            last_seq: 0,
            gap: Gap::default(),
            delay,
            stats: StreamStats::default(),
        }
    }

    /// Queues a packet that arrived at `now`.
    pub fn push(&mut self, packet: Packet<'_>, now: Instant) {
        self.stats.received += 1;
        let now = self.micros(now);

        if self.transits.is_empty() {
            self.newest_ts = packet.ts;
            self.newest_slot = 0;
        }
        let mut slot = self.slot_of(packet.ts);
        if let Some(base) = self.base_transit()
            && (now - slot * FRAME_MICROS - base).abs() > RESYNC_MICROS
        {
            self.restart(packet.ts);
            slot = 0;
        }
        if slot > self.newest_slot {
            self.newest_slot = slot;
            self.newest_ts = packet.ts;
        }

        if self.transits.len() == JITTER_WINDOW {
            self.transits.pop_front();
        }
        self.transits.push_back(now - slot * FRAME_MICROS);
        self.adapt_delay();

        if slot < self.next_slot {
            self.stats.late += 1;
            return;
        }
        if self.queue.len() == MAX_QUEUED {
            self.queue.pop_first();
        }
        self.queue.entry(slot).or_insert_with(|| QueuedPacket {
            seq: packet.seq,
            end_of_talk: packet.end_of_talk,
            payload: packet.payload.to_vec(),
        });
    }

    /// Produces the next 20 ms of audio for playout at `now`.
    pub fn pull(&mut self, out: &mut MonoFrame, now: Instant) -> Pulled {
        let Some(base) = self.base_transit() else {
            return silence(out);
        };
        let now = self.micros(now);
        let delay = self.delay.as_micros() as i64;
        // The slot whose playout time has most recently passed.
        let due_slot = (now - base - delay).div_euclid(FRAME_MICROS);

        if !self.playing {
            match self.queue.first_key_value() {
                Some((&first, packet)) if first <= due_slot => {
                    self.playing = true;
                    self.next_slot = first;
                    self.last_seq = packet.seq.wrapping_sub(1);
                }
                _ => return silence(out),
            }
        }

        // Stay within a frame of the schedule. Small timing noise is
        // ignored; drift between the sender's and our clocks, or a change of
        // delay, is fixed by stretching or skipping a frame.
        if self.next_slot > due_slot + 1 {
            self.conceal(out);
            return Pulled::Voice;
        }
        while self.next_slot < due_slot - 1 {
            self.queue.remove(&self.next_slot);
            self.next_slot += 1;
        }

        let slot = self.next_slot;
        self.next_slot += 1;
        if let Some(packet) = self.queue.remove(&slot) {
            self.play(&packet, out);
            return Pulled::Voice;
        }

        self.gap.frames += 1;
        if self.gap.frames > MAX_CONCEALED_RUN {
            self.end_talk_spurt();
            return silence(out);
        }
        match self.queue.first_key_value() {
            // Nothing between the last packet and the next was lost, so DTX
            // skipped this frame: the sender had nothing to say.
            Some((_, next)) if next.seq == self.last_seq.wrapping_add(1) => silence(out),
            Some((&next_slot, next)) if next_slot == slot + 1 => {
                if decode(&mut self.decoder, &next.payload, true, out) {
                    self.gap.fec_recovered += 1;
                } else {
                    self.conceal(out);
                    self.gap.concealed += 1;
                }
                Pulled::Voice
            }
            // Either more than one frame is missing, or no later packet has
            // arrived yet (which may also be the start of a DTX pause).
            _ => {
                self.conceal(out);
                self.gap.concealed += 1;
                Pulled::Voice
            }
        }
    }

    pub fn stats(&self) -> StreamStats {
        StreamStats {
            playout_delay: self.delay,
            ..self.stats.clone()
        }
    }

    fn play(&mut self, packet: &QueuedPacket, out: &mut MonoFrame) {
        // Count the frames filled since the last packet, up to the number of
        // packets that went missing; the rest were skipped by DTX.
        let lost = u64::from(packet.seq.wrapping_sub(self.last_seq).wrapping_sub(1));
        let fec_recovered = self.gap.fec_recovered.min(lost);
        self.stats.fec_recovered += fec_recovered;
        self.stats.concealed += self.gap.concealed.min(lost - fec_recovered);
        self.gap = Gap::default();
        self.last_seq = packet.seq;

        if !decode(&mut self.decoder, &packet.payload, false, out) {
            self.conceal(out);
            self.stats.concealed += 1;
        }
        if packet.end_of_talk {
            self.end_talk_spurt();
        }
    }

    fn conceal(&mut self, out: &mut MonoFrame) {
        if !decode(&mut self.decoder, &[], false, out) {
            out.fill(0.0);
        }
    }

    fn end_talk_spurt(&mut self) {
        self.playing = false;
        // A gap that no packet closes cannot be told apart from the sender
        // going quiet, so it is not counted as loss.
        self.gap = Gap::default();
        // The sender's encoder resets between spurts too.
        let _ = self.decoder.reset_state();
    }

    /// Forgets all timing after the sender restarted its clock, numbering
    /// `ts` as slot 0.
    fn restart(&mut self, ts: u32) {
        self.end_talk_spurt();
        self.queue.clear();
        self.transits.clear();
        self.next_slot = i64::MIN;
        self.newest_ts = ts;
        self.newest_slot = 0;
    }

    /// Sets the delay from the recent spread of transit times. It grows at
    /// once, but shrinks only between talk spurts, where nobody hears the
    /// skipped frames.
    fn adapt_delay(&mut self) {
        let mut transits: Vec<i64> = self.transits.iter().copied().collect();
        transits.sort_unstable();
        // Ignore the slowest 5%: waiting for rare stragglers would cost more
        // than concealing them.
        let typical_max = transits[(transits.len() - 1) * 95 / 100];
        let jitter = Duration::from_micros((typical_max - transits[0]) as u64);
        let wanted = (jitter + FRAME_DURATION)
            .max(self.config.target)
            .clamp(self.config.min, self.config.max);
        if wanted > self.delay || !self.playing {
            self.delay = wanted;
        }
    }

    /// The smallest recent transit time: the network's delay without jitter.
    fn base_transit(&self) -> Option<i64> {
        self.transits.iter().min().copied()
    }

    /// Numbers `ts` in frames relative to the newest packet, which keeps
    /// working when `ts` wraps around.
    fn slot_of(&self, ts: u32) -> i64 {
        let samples = i64::from(ts.wrapping_sub(self.newest_ts) as i32);
        let frame = FRAME_SAMPLES as i64;
        self.newest_slot + (samples + frame / 2).div_euclid(frame)
    }

    /// Microseconds since the first packet arrived.
    fn micros(&mut self, now: Instant) -> i64 {
        let epoch = *self.epoch.get_or_insert(now);
        now.saturating_duration_since(epoch).as_micros() as i64
    }
}

/// Decodes `payload` into `out`, or conceals a lost frame when `payload` is
/// empty. With `fec`, decodes the frame before `payload` from its FEC data.
/// Returns whether a whole frame came out.
fn decode(decoder: &mut opus::Decoder, payload: &[u8], fec: bool, out: &mut MonoFrame) -> bool {
    matches!(decoder.decode_float(payload, out, fec), Ok(FRAME_SAMPLES))
}

fn silence(out: &mut MonoFrame) -> Pulled {
    out.fill(0.0);
    Pulled::Silence
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::level_db;
    use crate::{Encoder, FRAME_SAMPLES};

    /// A sent packet, owning its payload.
    #[derive(Debug, Clone)]
    struct Sent {
        seq: u32,
        ts: u32,
        end_of_talk: bool,
        payload: Vec<u8>,
    }

    /// Encodes a 440 Hz tone, one frame per call.
    struct Talker {
        encoder: Encoder,
        seq: u32,
        ts: u32,
    }

    impl Talker {
        fn new() -> Self {
            Self::starting_at(1000)
        }

        fn starting_at(ts: u32) -> Self {
            Talker {
                encoder: Encoder::new(24_000).unwrap(),
                seq: 7,
                ts,
            }
        }

        fn frame(&mut self) -> Sent {
            let frame: MonoFrame = std::array::from_fn(|i| {
                let t = self.ts.wrapping_add(i as u32) as f32 / SAMPLE_RATE as f32;
                0.3 * (std::f32::consts::TAU * 440.0 * t).sin()
            });
            let payload = self.encoder.encode(&frame).unwrap().unwrap().to_vec();
            let sent = Sent {
                seq: self.seq,
                ts: self.ts,
                end_of_talk: false,
                payload,
            };
            self.seq = self.seq.wrapping_add(1);
            self.ts = self.ts.wrapping_add(FRAME_SAMPLES as u32);
            sent
        }

        /// Captures a frame that DTX decides not to send.
        fn skip_frame(&mut self) {
            self.ts = self.ts.wrapping_add(FRAME_SAMPLES as u32);
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Delivers each packet at its arrival time (relative to `start`) and
    /// pulls every 20 ms from `first_pull` on, like the mixer.
    fn simulate(
        stream: &mut ReceiveStream,
        arrivals: Vec<(Duration, Sent)>,
        first_pull: Duration,
        pulls: usize,
    ) -> Vec<Pulled> {
        simulate_from(stream, Instant::now(), arrivals, first_pull, pulls)
    }

    fn simulate_from(
        stream: &mut ReceiveStream,
        start: Instant,
        mut arrivals: Vec<(Duration, Sent)>,
        first_pull: Duration,
        pulls: usize,
    ) -> Vec<Pulled> {
        arrivals.sort_by_key(|(at, _)| *at);
        let mut arrivals = arrivals.into_iter().peekable();
        let mut out = [0.0; FRAME_SAMPLES];
        (0..pulls)
            .map(|i| {
                let pull_at = first_pull + FRAME_DURATION * i as u32;
                while let Some((_, sent)) = arrivals.next_if(|(at, _)| *at <= pull_at) {
                    let packet = Packet {
                        seq: sent.seq,
                        ts: sent.ts,
                        end_of_talk: sent.end_of_talk,
                        payload: &sent.payload,
                    };
                    stream.push(packet, start + pull_at);
                }
                stream.pull(&mut out, start + pull_at)
            })
            .collect()
    }

    /// A talk spurt: packets for `count` frames sent every 20 ms, each
    /// arriving after `delay(i)`. The last one has `end_of_talk` set.
    fn talk(
        talker: &mut Talker,
        count: usize,
        delay: impl Fn(usize) -> u64,
    ) -> Vec<(Duration, Sent)> {
        let mut spurt: Vec<_> = (0..count)
            .map(|i| (FRAME_DURATION * i as u32 + ms(delay(i)), talker.frame()))
            .collect();
        spurt.last_mut().unwrap().1.end_of_talk = true;
        spurt
    }

    /// Like [`talk`], but the sender goes quiet without `end_of_talk`, as
    /// when DTX kicks in.
    fn talk_then_dtx(talker: &mut Talker, count: usize) -> Vec<(Duration, Sent)> {
        let mut spurt = talk(talker, count, |_| 10);
        spurt.last_mut().unwrap().1.end_of_talk = false;
        spurt
    }

    fn voice_count(pulled: &[Pulled]) -> usize {
        pulled.iter().filter(|p| **p == Pulled::Voice).count()
    }

    /// Index of the first `Voice` and of the last one.
    fn voice_span(pulled: &[Pulled]) -> (usize, usize) {
        let first = pulled.iter().position(|p| *p == Pulled::Voice).unwrap();
        let last = pulled.iter().rposition(|p| *p == Pulled::Voice).unwrap();
        (first, last)
    }

    #[test]
    fn silent_before_any_packet() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        let mut out = [1.0; FRAME_SAMPLES];
        assert_eq!(stream.pull(&mut out, Instant::now()), Pulled::Silence);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn plays_in_order_stream_after_target_delay() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        let arrivals = talk(&mut Talker::new(), 50, |_| 30);
        let pulled = simulate(&mut stream, arrivals, ms(31), 60);
        // The first packet arrives at 30 ms and plays 40 ms later.
        let (first, last) = voice_span(&pulled);
        assert_eq!(first, 2);
        assert_eq!(last - first + 1, 50);
        assert_eq!(voice_count(&pulled), 50);
        let stats = stream.stats();
        assert_eq!(stats.received, 50);
        assert_eq!(
            (stats.late, stats.fec_recovered, stats.concealed),
            (0, 0, 0)
        );
        assert_eq!(stats.playout_delay, ms(40));
    }

    #[test]
    fn decoded_audio_matches_the_tone() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        let start = Instant::now();
        let mut talker = Talker::new();
        let mut out = [0.0; FRAME_SAMPLES];
        let mut decoded = Vec::new();
        for i in 0..40 {
            let at = start + FRAME_DURATION * i;
            let sent = talker.frame();
            let packet = Packet {
                seq: sent.seq,
                ts: sent.ts,
                end_of_talk: false,
                payload: &sent.payload,
            };
            stream.push(packet, at);
            if stream.pull(&mut out, at + ms(40)) == Pulled::Voice {
                decoded.extend_from_slice(&out);
            }
        }
        // Skip the codec's start-up.
        let steady = &decoded[FRAME_SAMPLES * 5..];
        let expected = 20.0 * (0.3f32 / 2f32.sqrt()).log10();
        assert!(
            (level_db(steady) - expected).abs() < 2.0,
            "{}",
            level_db(steady)
        );
        let crossings = steady
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count() as f32;
        let seconds = steady.len() as f32 / SAMPLE_RATE as f32;
        assert!((crossings / seconds - 880.0).abs() < 20.0, "{crossings}");
    }

    #[test]
    fn reorders_packets_within_the_delay() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        // Every pair arrives swapped: odd frames 5 ms early, even 15 ms late.
        let arrivals = talk(&mut Talker::new(), 50, |i| if i % 2 == 0 { 30 } else { 10 });
        let pulled = simulate(&mut stream, arrivals, ms(0), 70);
        assert_eq!(voice_count(&pulled), 50);
        let stats = stream.stats();
        assert_eq!(
            (stats.late, stats.fec_recovered, stats.concealed),
            (0, 0, 0)
        );
    }

    #[test]
    fn single_loss_is_recovered_by_fec() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        let mut arrivals = talk(&mut Talker::new(), 50, |_| 10);
        arrivals.remove(20);
        let pulled = simulate(&mut stream, arrivals, ms(0), 70);
        let (first, last) = voice_span(&pulled);
        assert_eq!(last - first + 1, 50, "no gap in playout");
        let stats = stream.stats();
        assert_eq!((stats.fec_recovered, stats.concealed), (1, 0));
    }

    #[test]
    fn burst_loss_is_concealed() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        let mut arrivals = talk(&mut Talker::new(), 50, |_| 10);
        arrivals.drain(20..24);
        let pulled = simulate(&mut stream, arrivals, ms(0), 70);
        let (first, last) = voice_span(&pulled);
        assert_eq!(last - first + 1, 50, "no gap in playout");
        assert_eq!(voice_count(&pulled), 50);
        let stats = stream.stats();
        // The last missing frame comes from the next packet's FEC data.
        assert_eq!((stats.fec_recovered, stats.concealed), (1, 3));
    }

    #[test]
    fn late_packet_is_dropped_and_covered() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        let arrivals = talk(&mut Talker::new(), 50, |i| if i == 20 { 400 } else { 10 });
        let pulled = simulate(&mut stream, arrivals, ms(0), 70);
        assert_eq!(voice_count(&pulled), 50);
        let stats = stream.stats();
        assert_eq!(stats.late, 1);
        assert_eq!(stats.fec_recovered, 1);
        // One outlier does not drag the delay to the maximum for long, but
        // it is bounded either way.
        assert!(stats.playout_delay <= ms(200));
    }

    #[test]
    fn delay_grows_with_jitter_within_bounds() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        // Up to 60 ms of jitter, deterministic.
        let arrivals = talk(&mut Talker::new(), 200, |i| (i as u64 * 37) % 61);
        let pulled = simulate(&mut stream, arrivals, ms(0), 230);
        let stats = stream.stats();
        assert!(stats.playout_delay >= ms(60), "{:?}", stats.playout_delay);
        assert!(stats.playout_delay <= ms(100), "{:?}", stats.playout_delay);
        // Once the delay covers the jitter nothing more is late.
        assert!(stats.late <= 5, "{stats:?}");
        assert!(voice_count(&pulled) >= 195);

        let mut stream = ReceiveStream::new(JitterConfig::default());
        let arrivals = talk(&mut Talker::new(), 100, |i| (i as u64 * 137) % 500);
        simulate(&mut stream, arrivals, ms(0), 150);
        assert_eq!(stream.stats().playout_delay, ms(200));
    }

    #[test]
    fn delay_shrinks_between_talk_spurts() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        let mut talker = Talker::new();
        // A jittery start, then enough steady packets to fill the window.
        let jitter = |i: usize| if i < 50 { (i as u64 * 37) % 61 } else { 10 };
        let arrivals = talk(&mut talker, 200, jitter);
        for _ in 0..10 {
            talker.skip_frame();
        }
        let next_spurt: Vec<_> = talk(&mut talker, 10, |_| 10)
            .into_iter()
            .map(|(at, s)| (at + ms(4200), s))
            .collect();

        let start = Instant::now();
        let pulled = simulate_from(&mut stream, start, arrivals, ms(0), 210);
        assert_eq!(
            *pulled.last().unwrap(),
            Pulled::Silence,
            "first spurt is over"
        );
        assert!(stream.stats().playout_delay >= ms(60));
        let pulled = simulate_from(&mut stream, start, next_spurt, ms(4200), 20);
        assert_eq!(voice_count(&pulled), 10);
        assert_eq!(stream.stats().playout_delay, ms(40));
    }

    #[test]
    fn end_of_talk_stops_playout_until_next_spurt() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        let mut talker = Talker::new();
        let mut arrivals = talk(&mut talker, 10, |_| 10);
        // Ten frames of push-to-talk released, then the next spurt.
        for _ in 0..10 {
            talker.skip_frame();
        }
        arrivals.extend(
            talk(&mut talker, 10, |_| 10)
                .into_iter()
                .map(|(at, s)| (at + ms(400), s)),
        );
        let pulled = simulate(&mut stream, arrivals, ms(0), 45);
        assert_eq!(voice_count(&pulled), 20);
        let silent_between = pulled[..30]
            .iter()
            .skip_while(|p| **p == Pulled::Silence)
            .skip(10)
            .take_while(|p| **p == Pulled::Silence)
            .count();
        assert_eq!(silent_between, 10);
        let stats = stream.stats();
        assert_eq!((stats.fec_recovered, stats.concealed), (0, 0));
    }

    #[test]
    fn dtx_gap_is_silence_not_loss() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        let mut talker = Talker::new();
        let mut arrivals = talk_then_dtx(&mut talker, 10);
        for _ in 0..3 {
            talker.skip_frame();
        }
        arrivals.extend(
            talk(&mut talker, 10, |_| 10)
                .into_iter()
                .map(|(at, s)| (at + ms(260), s)),
        );
        let pulled = simulate(&mut stream, arrivals, ms(0), 40);
        // The first skipped frame is concealed before the next packet shows
        // that nothing was lost; the others are silence.
        assert_eq!(voice_count(&pulled), 21);
        let stats = stream.stats();
        assert_eq!((stats.fec_recovered, stats.concealed), (0, 0));
    }

    #[test]
    fn spurt_without_end_of_talk_ends_after_concealing() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        let arrivals = talk_then_dtx(&mut Talker::new(), 10);
        let pulled = simulate(&mut stream, arrivals, ms(0), 30);
        assert_eq!(voice_count(&pulled), 10 + MAX_CONCEALED_RUN as usize);
        assert_eq!(*pulled.last().unwrap(), Pulled::Silence);
        // Nothing shows those frames were lost rather than never sent.
        assert_eq!(stream.stats().concealed, 0);
    }

    #[test]
    fn handles_ts_and_seq_wrapping() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        let mut talker = Talker::starting_at(u32::MAX - 5 * FRAME_SAMPLES as u32);
        talker.seq = u32::MAX - 3;
        let mut arrivals = talk(&mut talker, 20, |_| 10);
        arrivals.remove(8);
        let pulled = simulate(&mut stream, arrivals, ms(0), 30);
        assert_eq!(voice_count(&pulled), 20);
        assert_eq!(stream.stats().fec_recovered, 1);
    }

    #[test]
    fn restarted_sender_is_followed() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        let mut arrivals = talk(&mut Talker::new(), 10, |_| 10);
        let restarted = talk(&mut Talker::starting_at(2_000_000_000), 10, |_| 10);
        arrivals.extend(restarted.into_iter().map(|(at, s)| (at + ms(500), s)));
        let pulled = simulate(&mut stream, arrivals, ms(0), 45);
        assert!(voice_count(&pulled) >= 20, "{}", voice_count(&pulled));
    }

    #[test]
    fn duplicate_packets_are_ignored() {
        let mut stream = ReceiveStream::new(JitterConfig::default());
        let arrivals = talk(&mut Talker::new(), 20, |_| 10);
        let mut doubled = arrivals.clone();
        doubled.extend(arrivals.into_iter().map(|(at, s)| (at + ms(1), s)));
        let pulled = simulate(&mut stream, doubled, ms(0), 30);
        assert_eq!(voice_count(&pulled), 20);
        assert_eq!(stream.stats().received, 40);
    }
}
