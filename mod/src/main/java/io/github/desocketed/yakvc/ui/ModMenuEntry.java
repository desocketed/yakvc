package io.github.desocketed.yakvc.ui;

import com.terraformersmc.modmenu.api.ConfigScreenFactory;
import com.terraformersmc.modmenu.api.ModMenuApi;

/**
 * Mod Menu's config button, so the settings are reachable without binding the voice menu key. Fabric only loads this
 * class through the {@code modmenu} entrypoint, so the mod runs fine without Mod Menu installed.
 */
public final class ModMenuEntry implements ModMenuApi {
	@Override
	public ConfigScreenFactory<?> getModConfigScreenFactory() {
		return VoiceSettingsScreen::open;
	}
}
