package io.github.desocketed.yakvc.ui;

import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.VoiceSession;
import io.github.desocketed.yakvc.natives.EngineEvent;
import io.github.desocketed.yakvc.natives.NativeBridge;
import java.util.Locale;
import java.util.UUID;
import net.fabricmc.fabric.api.client.rendering.v1.hud.HudElement;
import net.minecraft.client.DeltaTracker;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphicsExtractor;

/**
 * A few lines of text in the top-left corner: voice state, then everyone talking, the local player included. Enough
 * to see that voice works until the real UI arrives (M6).
 */
public final class VoiceHud implements HudElement {
	private static final int GREY = 0xFFAAAAAA;
	private static final int GREEN = 0xFF55FF55;

	private final GameStateFeeder feeder;

	public VoiceHud(GameStateFeeder feeder) {
		this.feeder = feeder;
	}

	@Override
	public void extractRenderState(GuiGraphicsExtractor graphics, DeltaTracker deltaTracker) {
		Minecraft minecraft = Minecraft.getInstance();
		VoiceSession session = feeder.session();
		if (session == null || minecraft.player == null) return;
		Font font = minecraft.font;
		int y = 4;
		String status = status(session);
		if ((feeder.inputFlags() & NativeBridge.INPUT_MUTED) != 0) status += ", muted";
		if ((feeder.inputFlags() & NativeBridge.INPUT_DEAFENED) != 0) status += ", deafened";
		graphics.text(font, "Yak VC: " + status, 4, y, GREY, true);
		for (UUID uuid : feeder.talking()) {
			y += font.lineHeight + 1;
			String who = uuid.equals(minecraft.player.getUUID()) ? "You" : feeder.name(uuid);
			graphics.text(font, "> " + who, 4, y, GREEN, true);
		}
	}

	private String status(VoiceSession session) {
		if (feeder.closed()) return "off (engine stopped)";
		if (session.offReason() != null) return "off (" + session.offReason() + ")";
		if (!session.active()) return "waiting for server";
		EngineEvent.RendezvousState rendezvous = feeder.rendezvous();
		return rendezvous == null ? "starting" : rendezvous.name().toLowerCase(Locale.ROOT);
	}
}
