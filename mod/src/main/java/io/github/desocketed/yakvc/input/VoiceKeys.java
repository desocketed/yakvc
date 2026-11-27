package io.github.desocketed.yakvc.input;

import com.mojang.blaze3d.platform.InputConstants;
import net.fabricmc.fabric.api.client.keymapping.v1.KeyMappingHelper;
import net.minecraft.client.KeyMapping;
import net.minecraft.resources.Identifier;

/**
 * Voice keybinds. Push-to-talk defaults to V, which vanilla doesn't use. Mute, deafen and the debug overlay are
 * unbound by default so they don't clash with minimap and utility mods.
 */
public final class VoiceKeys {
	/** Holds push-to-talk down for dev and test runs that have no keyboard. */
	private static final boolean HOLD_PUSH_TO_TALK = Boolean.getBoolean("yakvc.dev.holdPushToTalk");

	public final KeyMapping pushToTalk;
	public final KeyMapping mute;
	public final KeyMapping deafen;
	public final KeyMapping debugOverlay;
	private boolean muted;
	private boolean deafened;

	private VoiceKeys(KeyMapping pushToTalk, KeyMapping mute, KeyMapping deafen, KeyMapping debugOverlay) {
		this.pushToTalk = pushToTalk;
		this.mute = mute;
		this.deafen = deafen;
		this.debugOverlay = debugOverlay;
	}

	public static VoiceKeys register() {
		KeyMapping.Category category = KeyMapping.Category.register(Identifier.fromNamespaceAndPath("yakvc", "voice"));
		return new VoiceKeys(
				KeyMappingHelper.registerKeyMapping(
						new KeyMapping("key.yakvc.push_to_talk", InputConstants.KEY_V, category)),
				KeyMappingHelper.registerKeyMapping(
						new KeyMapping("key.yakvc.mute", InputConstants.UNKNOWN.getValue(), category)),
				KeyMappingHelper.registerKeyMapping(
						new KeyMapping("key.yakvc.deafen", InputConstants.UNKNOWN.getValue(), category)),
				KeyMappingHelper.registerKeyMapping(
						new KeyMapping("key.yakvc.debug_overlay", InputConstants.UNKNOWN.getValue(), category)));
	}

	/** Applies mute and deafen presses since the last tick. */
	public void tick() {
		while (mute.consumeClick()) muted = !muted;
		while (deafen.consumeClick()) deafened = !deafened;
	}

	/** Whether the debug overlay key was pressed since the last call. */
	public boolean debugOverlayPressed() {
		return pressed(debugOverlay);
	}

	private static boolean pressed(KeyMapping key) {
		boolean pressed = false;
		while (key.consumeClick()) pressed = true;
		return pressed;
	}

	public boolean pushToTalkDown() {
		return HOLD_PUSH_TO_TALK || pushToTalk.isDown();
	}

	public boolean muted() {
		return muted;
	}

	public boolean deafened() {
		return deafened;
	}
}
