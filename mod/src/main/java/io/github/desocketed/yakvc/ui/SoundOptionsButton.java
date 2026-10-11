package io.github.desocketed.yakvc.ui;

import java.util.List;
import net.fabricmc.fabric.api.client.screen.v1.ScreenEvents;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.AbstractButton;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.OptionsList;
import net.minecraft.client.gui.components.events.ContainerEventHandler;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.screens.options.SoundOptionsScreen;
import net.minecraft.network.chat.Component;

/**
 * A "Voice Chat…" button at the end of the game's Music &amp; Sounds options, where players look for audio settings
 * first. It goes into the screen's own options list, so it scrolls with the other options and never overlaps them.
 */
public final class SoundOptionsButton {
	private static final Component LABEL = Component.translatable("yakvc.sound_options.voice_chat");

	private SoundOptionsButton() {
	}

	public static void register() {
		// This runs whenever the screen is initialised: when it opens, on resize, and when a screen on top of it (such
		// as the voice settings) closes. The options list can survive that, so the button is added only once.
		ScreenEvents.AFTER_INIT.register((minecraft, screen, width, height) -> {
			if (!(screen instanceof SoundOptionsScreen)) return;
			for (GuiEventListener widget : screen.children()) {
				if (widget instanceof OptionsList list && !hasButton(list.children())) {
					list.addBig(Button.builder(LABEL,
							button -> Minecraft.getInstance().gui.setScreen(VoiceSettingsScreen.open(screen))).build());
				}
			}
		});
	}

	private static boolean hasButton(List<? extends GuiEventListener> widgets) {
		for (GuiEventListener widget : widgets) {
			if (widget instanceof AbstractButton button && button.getMessage().equals(LABEL)) return true;
			if (widget instanceof ContainerEventHandler container && hasButton(container.children())) return true;
		}
		return false;
	}
}
