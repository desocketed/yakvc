package io.github.desocketed.yakvc.ui;

import java.util.function.DoubleSupplier;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.AbstractWidget;
import net.minecraft.client.gui.narration.NarrationElementOutput;
import net.minecraft.network.chat.Component;

/**
 * A bar showing the microphone's live level, so players can see it working without joining a world. A white mark shows
 * the voice activation threshold; the bar turns green above it, where voice activation would transmit.
 */
final class MicLevelMeter extends AbstractWidget {
	/** The quietest level the bar shows; speech sits well above it, and the default threshold (-45) fits. */
	private static final double FLOOR_DB = -60;

	private final DoubleSupplier levelDb;
	private final double thresholdDb;

	MicLevelMeter(DoubleSupplier levelDb, double thresholdDb) {
		super(0, 0, 310, 20, Component.translatable("yakvc.settings.mic_level"));
		this.levelDb = levelDb;
		this.thresholdDb = thresholdDb;
		active = false;
	}

	@Override
	protected void extractWidgetRenderState(GuiGraphicsExtractor graphics, int mouseX, int mouseY, float a) {
		int left = getX();
		int top = getY() + 11;
		int right = getRight();
		int bottom = getBottom();
		double level = levelDb.getAsDouble();
		graphics.text(Minecraft.getInstance().font, getMessage(), left, getY(), 0xFFFFFFFF);
		graphics.fill(left, top, right, bottom, 0xFF000000);
		graphics.fill(left + 1, top + 1, x(level, left + 1, right - 1), bottom - 1,
				level > thresholdDb ? 0xFF55FF55 : 0xFFAAAAAA);
		int mark = x(thresholdDb, left + 1, right - 1);
		graphics.fill(mark, top, mark + 1, bottom, 0xFFFFFFFF);
	}

	/** Where {@code db} falls between {@code left} (the floor) and {@code right} (0 dBFS). */
	private static int x(double db, int left, int right) {
		double fraction = Math.clamp((db - FLOOR_DB) / -FLOOR_DB, 0, 1);
		return left + (int) Math.round(fraction * (right - left));
	}

	@Override
	protected void updateWidgetNarration(NarrationElementOutput output) {
		// A level changing ten times a second isn't worth reading aloud.
	}
}
