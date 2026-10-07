package io.github.desocketed.yakvc.ui;

import net.fabricmc.fabric.api.client.screen.v1.ScreenEvents;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.OptionsList;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.screens.options.SoundOptionsScreen;
import net.minecraft.network.chat.Component;

/**
 * A "Voice Chat…" button at the end of the game's Music &amp; Sounds options, where players look for audio settings
 * first. It goes into the screen's own options list, so it scrolls with the other options and never overlaps them.
 */
public final class SoundOptionsButton {
	private SoundOptionsButton() {
	}

	public static void register() {
		// The screen is rebuilt, and this runs again, whenever the window is resized.
		ScreenEvents.AFTER_INIT.register((minecraft, screen, width, height) -> {
			if (!(screen instanceof SoundOptionsScreen)) return;
			for (GuiEventListener widget : screen.children()) {
				if (widget instanceof OptionsList list) {
					list.addBig(Button.builder(Component.translatable("yakvc.sound_options.voice_chat"),
							button -> Minecraft.getInstance().gui.setScreen(VoiceSettingsScreen.open(screen))).build());
				}
			}
		});
	}
}
