package io.github.desocketed.yakvc.ui;

import com.terraformersmc.modmenu.api.ConfigScreenFactory;
import com.terraformersmc.modmenu.api.ModMenuApi;
import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.YakVcClient;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.AlertScreen;
import net.minecraft.network.chat.Component;

/**
 * Mod Menu's config button, so the settings are reachable without binding the voice menu key. Fabric only loads this
 * class through the {@code modmenu} entrypoint, so the mod runs fine without Mod Menu installed.
 */
public final class ModMenuEntry implements ModMenuApi {
	@Override
	public ConfigScreenFactory<?> getModConfigScreenFactory() {
		return parent -> {
			Minecraft minecraft = Minecraft.getInstance();
			GameStateFeeder feeder = YakVcClient.feeder();
			if (feeder == null) {
				// The settings need the running engine to check and apply changes.
				return new AlertScreen(() -> minecraft.gui.setScreen(parent), Component.translatable("yakvc.settings.title"),
						Component.translatable("yakvc.settings.unavailable"));
			}
			return new VoiceSettingsScreen(parent, minecraft.options, feeder);
		};
	}
}
