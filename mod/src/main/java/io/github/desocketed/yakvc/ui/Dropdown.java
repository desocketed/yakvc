package io.github.desocketed.yakvc.ui;

import java.util.List;
import java.util.function.Function;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.AbstractButton;
import net.minecraft.client.input.InputWithModifiers;
import net.minecraft.client.gui.narration.NarrationElementOutput;
import net.minecraft.network.chat.Component;

/**
 * A button that opens a scrollable list of values below (or above) itself; picking one closes it. Minecraft has no
 * dropdown, and cycling through a long list of audio devices is slow.
 *
 * <p>The open list is drawn over the rest of the screen, outside the scrolling options list that clips its widgets,
 * so the screen passes its own rendering and input on to {@link #extractList}, {@link #listClicked} and
 * {@link #listScrolled}.
 */
final class Dropdown extends AbstractButton {
	private static final int ROW_HEIGHT = 12;
	private static final int MAX_ROWS = 8;

	private final Component caption;
	private final List<String> values;
	private final Function<String, Component> label;
	private String selected;
	private boolean open;
	/** The first visible row. */
	private int scroll;

	Dropdown(Component caption, List<String> values, String selected, Function<String, Component> label) {
		super(0, 0, 310, 20, Component.empty());
		this.caption = caption;
		this.values = values;
		this.label = label;
		select(selected);
	}

	String get() {
		return selected;
	}

	boolean isOpen() {
		return open;
	}

	void close() {
		open = false;
	}

	private void select(String value) {
		selected = value;
		setMessage(Options.genericValueLabel(caption, Component.empty().append(label.apply(value)).append(" ▼")));
	}

	@Override
	public void onPress(InputWithModifiers input) {
		open = !open;
		// Start with the selected value in view.
		scroll = Math.clamp(values.indexOf(selected) - MAX_ROWS / 2, 0, maxScroll());
	}

	@Override
	protected void extractContents(GuiGraphicsExtractor graphics, int mouseX, int mouseY, float a) {
		extractDefaultSprite(graphics);
		extractDefaultLabel(graphics.textRendererForWidget(this, GuiGraphicsExtractor.HoveredTextEffects.NONE));
	}

	private int visibleRows() {
		return Math.min(values.size(), MAX_ROWS);
	}

	private int maxScroll() {
		return values.size() - visibleRows();
	}

	/** Below the button if it fits on the screen, otherwise above. */
	private int listTop(int screenHeight) {
		int height = visibleRows() * ROW_HEIGHT + 2;
		return getBottom() + height <= screenHeight ? getBottom() : getY() - height;
	}

	/** The row under the point, or -1. */
	private int rowAt(double x, double y, int screenHeight) {
		int top = listTop(screenHeight) + 1;
		if (x < getX() || x >= getRight() || y < top || y >= top + visibleRows() * ROW_HEIGHT) return -1;
		return scroll + (int) ((y - top) / ROW_HEIGHT);
	}

	void extractList(GuiGraphicsExtractor graphics, int mouseX, int mouseY) {
		if (!open) return;
		// A new stratum draws over everything already drawn, including other widgets' text.
		graphics.nextStratum();
		Font font = Minecraft.getInstance().font;
		int top = listTop(graphics.guiHeight());
		int bottom = top + visibleRows() * ROW_HEIGHT + 2;
		graphics.fill(getX(), top, getRight(), bottom, 0xFF000000);
		graphics.outline(getX(), top, getWidth(), bottom - top, 0xFFA0A0A0);
		int hovered = rowAt(mouseX, mouseY, graphics.guiHeight());
		for (int i = 0; i < visibleRows(); i++) {
			int row = scroll + i;
			int y = top + 1 + i * ROW_HEIGHT;
			if (row == hovered) graphics.fill(getX() + 1, y, getRight() - 1, y + ROW_HEIGHT, 0xFF404040);
			String text = font.plainSubstrByWidth(label.apply(values.get(row)).getString(), getWidth() - 12);
			int color = values.get(row).equals(selected) ? 0xFFFFFF55 : 0xFFFFFFFF;
			graphics.text(font, text, getX() + 4, y + 2, color);
		}
		if (maxScroll() > 0) {
			// A thumb on the right edge, so players can tell there is more to scroll to.
			int track = bottom - top - 2;
			int thumb = Math.max(4, track * visibleRows() / values.size());
			int thumbTop = top + 1 + (track - thumb) * scroll / maxScroll();
			graphics.fill(getRight() - 3, thumbTop, getRight() - 1, thumbTop + thumb, 0xFFA0A0A0);
		}
	}

	/** Picks the clicked row and closes. A click anywhere else just closes; either way the click is used up. */
	boolean listClicked(double x, double y, int screenHeight) {
		if (!open) return false;
		int row = rowAt(x, y, screenHeight);
		if (row >= 0) select(values.get(row));
		open = false;
		return true;
	}

	/** Scrolls the open list; other scrolling would move the button away from its list, so it is used up too. */
	boolean listScrolled(double amount) {
		if (!open) return false;
		scroll = Math.clamp(scroll - (int) Math.signum(amount), 0, maxScroll());
		return true;
	}

	@Override
	protected void updateWidgetNarration(NarrationElementOutput output) {
		defaultButtonNarrationText(output);
	}
}
