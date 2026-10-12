package io.github.desocketed.yakvc.ui;

import io.github.desocketed.yakvc.UpdateCheck;
import java.util.List;
import net.fabricmc.fabric.api.client.screen.v1.ScreenEvents;
import net.fabricmc.fabric.api.client.screen.v1.Screens;
import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.AbstractWidget;
import net.minecraft.client.gui.components.PlainTextButton;
import net.minecraft.client.gui.screens.ConfirmLinkScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;

/**
 * A line of text in the title screen's top left corner naming a newer Yak VC, which opens its release page (after the
 * game's usual link warning) when clicked. Only the title screen: in game it would be in the way.
 */
public final class UpdateNotice {
	private static final int MARGIN = 4;

	private static UpdateCheck.@Nullable Release newer;

	private UpdateNotice() {}

	public static void register() {
		ScreenEvents.AFTER_INIT.register((minecraft, screen, width, height) -> addTo(screen));
	}

	/** The newer release the update check found, or null. */
	public static UpdateCheck.@Nullable Release newer() {
		return newer;
	}

	/** Shows {@code release} from now on. Must run on the client thread. */
	public static void show(UpdateCheck.Release release) {
		newer = release;
		// The check usually finishes while the title screen is already up, after its last init.
		Screen screen = Minecraft.getInstance().gui.screen();
		if (screen != null) addTo(screen);
	}

	private static void addTo(Screen screen) {
		UpdateCheck.Release release = newer;
		if (release == null || !(screen instanceof TitleScreen)) return;
		List<AbstractWidget> widgets = Screens.getWidgets(screen);
		Component text = Component.translatable("yakvc.update.available", release.version())
				.withStyle(ChatFormatting.UNDERLINE);
		for (AbstractWidget widget : widgets) {
			if (widget.getMessage().equals(text)) return;
		}
		var font = Minecraft.getInstance().font;
		widgets.add(new PlainTextButton(MARGIN, MARGIN, font.width(text), font.lineHeight, text,
				button -> ConfirmLinkScreen.confirmLinkNow(screen, release.page()), font));
	}
}
