package io.github.desocketed.yakvc.input;

import com.mojang.blaze3d.platform.InputConstants;
import net.fabricmc.fabric.api.client.keymapping.v1.KeyMappingHelper;
import net.minecraft.client.KeyMapping;
import net.minecraft.resources.Identifier;

/**
 * Voice keybinds. Push-to-talk defaults to V, which vanilla doesn't use. Mute and deafen are unbound by default so they
 * don't clash with minimap and utility mods.
 */
public final class VoiceKeys {
	/** Holds push-to-talk down for dev and test runs that have no keyboard. */
	private static final boolean HOLD_PUSH_TO_TALK = Boolean.getBoolean("yakvc.dev.holdPushToTalk");

	public final KeyMapping pushToTalk;
	public final KeyMapping mute;
	public final KeyMapping deafen;
	private boolean muted;
	private boolean deafened;

	private VoiceKeys(KeyMapping pushToTalk, KeyMapping mute, KeyMapping deafen) {
		this.pushToTalk = pushToTalk;
		this.mute = mute;
		this.deafen = deafen;
	}

	public static VoiceKeys register() {
		KeyMapping.Category category = KeyMapping.Category.register(Identifier.fromNamespaceAndPath("yakvc", "voice"));
		return new VoiceKeys(
				KeyMappingHelper.registerKeyMapping(
						new KeyMapping("key.yakvc.push_to_talk", InputConstants.KEY_V, category)),
				KeyMappingHelper.registerKeyMapping(
						new KeyMapping("key.yakvc.mute", InputConstants.UNKNOWN.getValue(), category)),
				KeyMappingHelper.registerKeyMapping(
						new KeyMapping("key.yakvc.deafen", InputConstants.UNKNOWN.getValue(), category)));
	}

	/** Applies mute and deafen presses since the last tick. */
	public void tick() {
		while (mute.consumeClick()) muted = !muted;
		while (deafen.consumeClick()) deafened = !deafened;
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
