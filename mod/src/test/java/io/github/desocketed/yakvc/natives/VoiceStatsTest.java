package io.github.desocketed.yakvc.natives;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;

import java.util.UUID;
import org.junit.jupiter.api.Test;

class VoiceStatsTest {
	private static final UUID A = UUID.fromString("00112233-4455-6677-8899-aabbccddeeff");
	private static final UUID B = UUID.fromString("ffeeddcc-bbaa-9988-7766-554433221100");

	@Test
	void parsesTheSnapshot() {
		VoiceStats stats = VoiceStats.parse("""
				{"endpoint_id": "ab12", "voice_version": 4, "audio": {"microphone_open": true, "speakers_open": false,
				 "transmitting": true, "bitrate": 16000, "overruns": 2, "underruns": 3},
				 "peers": [
				  {"uuid": "00112233-4455-6677-8899-aabbccddeeff", "rtt_ms": 12.5, "voice_version": 3, "received": 95, "late": 5,
				   "fec_recovered": 4, "concealed": 6, "playout_delay_ms": 60.0},
				  {"uuid": "ffeeddcc-bbaa-9988-7766-554433221100", "rtt_ms": null}]}
				""");
		assertEquals("ab12", stats.endpointId());
		assertEquals(4, stats.voiceVersion());
		assertEquals(new VoiceStats.Audio(true, false, true, 16000, 2, 3), stats.audio());
		assertEquals(new VoiceStats.Peer(12.5, new VoiceStats.Stream(95, 5, 4, 6, 60.0), 3), stats.peers().get(A));
		assertEquals(new VoiceStats.Peer(null, null, null), stats.peers().get(B));
		// 10 lost of 90 on time plus 10 lost.
		assertEquals(10.0, stats.peers().get(A).stream().lossPercent(), 1e-9);
	}

	@Test
	void noFramesIsNoLoss() {
		assertEquals(0.0, new VoiceStats.Stream(0, 0, 0, 0, 0).lossPercent());
		assertNull(new VoiceStats.Peer(null, null, null).rttMs());
	}
}
