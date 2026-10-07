package io.github.desocketed.yakvc.natives;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.util.HashMap;
import java.util.Map;
import java.util.UUID;
import org.jspecify.annotations.Nullable;

/** The engine's diagnostics snapshot from {@code yakvc_stats}, for the debug overlay. */
public record VoiceStats(String endpointId, Audio audio, Map<UUID, Peer> peers) {
	/**
	 * The audio thread. {@code bitrate} is what the encoder uses now, which is lower than configured while the relay
	 * budget is tight. Overruns and underruns count from when the device was last opened.
	 */
	public record Audio(boolean microphoneOpen, boolean speakersOpen, boolean transmitting, int bitrate, long overruns,
			long underruns) {}

	/** A connected peer. {@code rttMs} is null until the path has one; {@code stream} until audio arrived. */
	public record Peer(@Nullable Double rttMs, @Nullable Stream stream) {}

	/** Receive counters in 20 ms frames, and the jitter buffer's current delay. */
	public record Stream(long received, long late, long fecRecovered, long concealed, double playoutDelayMs) {
		/**
		 * Percent of the frames the peer sent that never arrived in time. FEC-recovered frames count as lost: they are
		 * still a sign of a lossy path.
		 */
		public double lossPercent() {
			long lost = fecRecovered + concealed;
			long expected = received - late + lost;
			return expected <= 0 ? 0 : 100.0 * lost / expected;
		}
	}

	public static VoiceStats parse(String json) {
		JsonObject root = JsonParser.parseString(json).getAsJsonObject();
		JsonObject a = root.getAsJsonObject("audio");
		Audio audio = new Audio(a.get("microphone_open").getAsBoolean(), a.get("speakers_open").getAsBoolean(),
				a.get("transmitting").getAsBoolean(), a.get("bitrate").getAsInt(), a.get("overruns").getAsLong(),
				a.get("underruns").getAsLong());
		Map<UUID, Peer> peers = new HashMap<>();
		for (JsonElement element : root.getAsJsonArray("peers")) {
			JsonObject p = element.getAsJsonObject();
			JsonElement rtt = p.get("rtt_ms");
			Stream stream = p.has("received")
					? new Stream(p.get("received").getAsLong(), p.get("late").getAsLong(),
							p.get("fec_recovered").getAsLong(), p.get("concealed").getAsLong(),
							p.get("playout_delay_ms").getAsDouble())
					: null;
			peers.put(UUID.fromString(p.get("uuid").getAsString()),
					new Peer(rtt == null || rtt.isJsonNull() ? null : rtt.getAsDouble(), stream));
		}
		return new VoiceStats(root.get("endpoint_id").getAsString(), audio, peers);
	}
}
