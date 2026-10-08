package io.github.desocketed.yakvc.ui;

import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.VoiceSession;
import io.github.desocketed.yakvc.natives.EngineEvent;
import io.github.desocketed.yakvc.natives.NativeBridge;
import java.util.UUID;
import net.fabricmc.fabric.api.client.rendering.v1.hud.HudElement;
import net.minecraft.client.DeltaTracker;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;

/**
 * Top-left corner, while in a world: the own microphone (green while sending, red when muted, grey when voice is off
 * or the microphone isn't working), a red headphones icon when deafened, and the rendezvous connection as signal bars (green when registered). A short
 * line of text follows only when something is not working. Below, one line per talking player, for players outside
 * the view; players in view also get {@link TalkingIndicator} over their heads.
 */
public final class VoiceHud implements HudElement {
	private static final int WHITE = 0xFFFFFFFF;
	private static final int GREY = 0xFF888888;
	private static final int GREEN = 0xFF55FF55;
	private static final int YELLOW = 0xFFFFFF55;
	private static final int RED = 0xFFFF5555;

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
		int flags = feeder.inputFlags();
		Component problem = problem(feeder);
		// Not a reason voice is off, as the others are: the player can still hear.
		if (problem == null && !feeder.micWorking()) problem = Component.translatable("yakvc.hud.no_microphone");

		int x = 4;
		int y = 4;
		int micColor;
		if (problem != null) micColor = GREY;
		else if ((flags & NativeBridge.INPUT_MUTED) != 0) micColor = RED;
		else if (feeder.talking().contains(minecraft.player.getUUID())) micColor = GREEN;
		else micColor = WHITE;
		x = icon(graphics, font, Icons.MIC, x, y, micColor);
		if ((flags & NativeBridge.INPUT_DEAFENED) != 0) x = icon(graphics, font, Icons.HEADPHONES, x, y, RED);
		x = icon(graphics, font, Icons.SIGNAL, x, y, signalColor(feeder.rendezvous()));
		if (problem != null) graphics.text(font, problem, x, y, GREY, true);

		for (UUID uuid : feeder.talking()) {
			if (uuid.equals(minecraft.player.getUUID())) continue;
			y += font.lineHeight + 1;
			int nameX = icon(graphics, font, Icons.SPEAKER, 4, y, WHITE);
			graphics.text(font, feeder.name(uuid), nameX, y, WHITE, true);
		}
	}

	/** Draws an icon and returns the x after it and a gap. */
	private static int icon(GuiGraphicsExtractor graphics, Font font, Component icon, int x, int y, int color) {
		graphics.text(font, icon, x, y, color, true);
		return x + font.width(icon) + 3;
	}

	private static int signalColor(EngineEvent.@Nullable RendezvousState state) {
		if (state == EngineEvent.RendezvousState.REGISTERED) return GREEN;
		if (state == EngineEvent.RendezvousState.DISCONNECTED) return RED;
		return YELLOW;
	}

	/** Why voice isn't working in this world right now, or null if it is (or there is no world). */
	static @Nullable Component problem(GameStateFeeder feeder) {
		VoiceSession session = feeder.session();
		if (feeder.closed()) return Component.translatable("yakvc.hud.stopped");
		if (session == null) return null;
		if (session.offReason() != null) return Component.translatable("yakvc.hud.off", session.offReason());
		if (!session.active()) return Component.translatable("yakvc.hud.waiting");
		return null;
	}
}
