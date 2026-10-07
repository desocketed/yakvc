package io.github.desocketed.yakvc.ui;

import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.config.ClientConfig;
import io.github.desocketed.yakvc.natives.EngineEvent;
import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.natives.VoiceStats;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import net.fabricmc.fabric.api.client.rendering.v1.hud.HudElement;
import net.minecraft.client.DeltaTracker;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import org.jspecify.annotations.Nullable;

/**
 * Voice diagnostics in the top-right corner, in the spirit of F3, for testing and bug reports. Toggled by its key or
 * in the voice settings. The text is English only, like F3, so screenshots read the same in every bug report.
 */
public final class DebugOverlay implements HudElement {
	private static final int TEXT = 0xFFE0E0E0;
	private static final int BACKGROUND = 0x90505050;

	private final GameStateFeeder feeder;

	public DebugOverlay(GameStateFeeder feeder) {
		this.feeder = feeder;
	}

	@Override
	public void extractRenderState(GuiGraphicsExtractor graphics, DeltaTracker deltaTracker) {
		if (!feeder.debugOverlay()) return;
		Font font = Minecraft.getInstance().font;
		int y = 2;
		for (String line : lines()) {
			int width = font.width(line);
			int x = graphics.guiWidth() - 2 - width;
			graphics.fill(x - 1, y - 1, x + width + 1, y + font.lineHeight - 1, BACKGROUND);
			graphics.text(font, line, x, y, TEXT, false);
			y += font.lineHeight;
		}
	}

	private List<String> lines() {
		List<String> lines = new ArrayList<>();
		lines.add("Yak VC");
		if (feeder.closed()) {
			lines.add("Engine stopped");
			return lines;
		}
		ClientConfig config = feeder.config();
		VoiceStats stats = feeder.stats();

		lines.add("Rendezvous: " + rendezvous());
		lines.add("Account: " + (feeder.verified() ? "verified" : "not verified"));
		lines.add("Endpoint: " + (stats == null ? "…" : stats.endpointId().substring(0, 10))
				+ (config.relayOnly() ? ", relay only" : ""));

		lines.add("");
		lines.add("Microphone: " + deviceName(config.inputDevice(), "default")
				+ (stats == null ? "" : stats.audio().microphoneOpen() ? " (open)" : " (not open)"));
		lines.add("Speakers: " + deviceName(config.outputDevice(), "same as game")
				+ (stats == null ? "" : stats.audio().speakersOpen() ? " (open)" : " (not open)"));
		lines.add(String.format("Level: %.0f dBFS, %s", feeder.micLevelDb(),
				config.voiceActivation() ? String.format("voice above %.0f dBFS", config.vadThresholdDb())
						: "push to talk"));
		int flags = feeder.inputFlags();
		String input = (flags & NativeBridge.INPUT_MUTED) != 0 ? ", muted" : "";
		if ((flags & NativeBridge.INPUT_DEAFENED) != 0) input += ", deafened";
		if (stats != null) {
			VoiceStats.Audio audio = stats.audio();
			lines.add((audio.transmitting() ? "Sending" : "Not sending") + input);
			lines.add(String.format("Bitrate: %d kbit/s (set %d)", audio.bitrate() / 1000, config.bitrate() / 1000));
			lines.add("Overruns: " + audio.overruns() + ", underruns: " + audio.underruns());
		}

		Map<UUID, VoiceStats.Peer> peerStats = stats == null ? Map.of() : stats.peers();
		lines.add("");
		lines.add("Peers: " + peerStats.size());
		for (Map.Entry<UUID, VoiceStats.Peer> entry : peerStats.entrySet()) {
			lines.add(peer(entry.getKey(), entry.getValue()));
		}
		return lines;
	}

	private String rendezvous() {
		EngineEvent.RendezvousState state = feeder.rendezvous();
		if (state == null) return "not started";
		long value = feeder.rendezvousValue();
		return switch (state) {
			case REGISTERED -> {
				long minutes = Math.max(0, value - System.currentTimeMillis() / 1000) / 60;
				yield String.format("registered, ticket expires in %dh %02dm", minutes / 60, minutes % 60);
			}
			case RETRYING -> String.format("retrying in %.1f s", value / 1000.0);
			default -> state.name().toLowerCase();
		};
	}

	/** One peer: name, path, round trip, loss and jitter buffer delay. */
	private String peer(UUID uuid, VoiceStats.Peer stats) {
		EngineEvent.PeerState state = feeder.peerState(uuid);
		StringBuilder line = new StringBuilder(feeder.name(uuid)).append(": ")
				.append(state == null ? "?" : state.name().toLowerCase().replace('_', ' '));
		if (feeder.peerVerified(uuid)) line.append(", verified");
		if (stats.rttMs() != null) line.append(String.format(", rtt %.0f ms", stats.rttMs()));
		VoiceStats.Stream stream = stats.stream();
		if (stream != null) {
			line.append(String.format(", loss %.1f%%, late %d, buffer %.0f ms", stream.lossPercent(), stream.late(),
					stream.playoutDelayMs()));
		}
		return line.toString();
	}

	private static String deviceName(@Nullable String configured, String unset) {
		return configured == null ? unset : configured;
	}
}
