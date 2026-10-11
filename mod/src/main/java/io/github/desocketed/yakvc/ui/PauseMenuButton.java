package io.github.desocketed.yakvc.ui;

import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.YakVcClient;
import net.fabricmc.fabric.api.client.screen.v1.ScreenEvents;
import net.fabricmc.fabric.api.client.screen.v1.Screens;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.AbstractButton;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.Tooltip;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.screens.PauseScreen;
import net.minecraft.network.chat.Component;

/**
 * A microphone button next to "Back to Game" in the pause menu, opening the voice menu. The voice menu's key is
 * unbound by default, so this is how most players reach it; the HUD icon can't be clicked while the mouse is captured.
 */
public final class PauseMenuButton {
	private static final Component BACK_TO_GAME = Component.translatable("menu.returnToGame");
	private static final Component TOOLTIP = Component.translatable("yakvc.pause.voice_chat");
	private static final int SIZE = 20;
	private static final int GAP = 4;

	private PauseMenuButton() {
	}

	public static void register() {
		ScreenEvents.AFTER_INIT.register((minecraft, screen, width, height) -> {
			if (!(screen instanceof PauseScreen pause) || !pause.showsPauseMenu()) return;
			GameStateFeeder feeder = YakVcClient.feeder();
			AbstractButton back = backToGame(screen.children());
			if (feeder == null || back == null) return;
			Screens.getWidgets(screen).add(Button.builder(Icons.MIC,
							button -> Minecraft.getInstance().gui.setScreen(new VoiceMenuScreen(screen, feeder)))
					.bounds(back.getX() + back.getWidth() + GAP, back.getY(), SIZE, SIZE)
					.tooltip(Tooltip.create(TOOLTIP))
					.build());
		});
	}

	private static AbstractButton backToGame(Iterable<? extends GuiEventListener> widgets) {
		for (GuiEventListener widget : widgets) {
			if (widget instanceof AbstractButton button && button.getMessage().equals(BACK_TO_GAME)) return button;
		}
		return null;
	}
}
