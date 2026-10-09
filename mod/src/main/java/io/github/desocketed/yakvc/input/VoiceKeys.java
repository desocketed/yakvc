package io.github.desocketed.yakvc.input;

import com.mojang.blaze3d.platform.InputConstants;
import net.fabricmc.fabric.api.client.keymapping.v1.KeyMappingHelper;
import net.minecraft.client.KeyMapping;
import net.minecraft.resources.Identifier;

/**
 * Voice keybinds. Push-to-talk defaults to V and Group to B, which vanilla doesn't use. Mute, deafen and the voice
 * menu are unbound by default so they don't clash with minimap and utility mods; so is the debug overlay.
 */
public final class VoiceKeys {
	/** Holds push-to-talk down for dev and test runs that have no keyboard. */
	private static final boolean HOLD_PUSH_TO_TALK = Boolean.getBoolean("yakvc.dev.holdPushToTalk");

	public final KeyMapping pushToTalk;
	/** Invites the player looked at to a group, or accepts their invite. */
	public final KeyMapping group;
	public final KeyMapping mute;
	public final KeyMapping deafen;
	public final KeyMapping openMenu;
	public final KeyMapping debugOverlay;
	private boolean muted;
	private boolean deafened;

	private VoiceKeys(KeyMapping pushToTalk, KeyMapping group, KeyMapping mute, KeyMapping deafen,
			KeyMapping openMenu, KeyMapping debugOverlay) {
		this.pushToTalk = pushToTalk;
		this.group = group;
		this.mute = mute;
		this.deafen = deafen;
		this.openMenu = openMenu;
		this.debugOverlay = debugOverlay;
	}

	public static VoiceKeys register() {
		KeyMapping.Category category = KeyMapping.Category.register(Identifier.fromNamespaceAndPath("yakvc", "voice"));
		return new VoiceKeys(
				KeyMappingHelper.registerKeyMapping(
						new KeyMapping("key.yakvc.push_to_talk", InputConstants.KEY_V, category)),
				KeyMappingHelper.registerKeyMapping(
						new KeyMapping("key.yakvc.group", InputConstants.KEY_B, category)),
				KeyMappingHelper.registerKeyMapping(
						new KeyMapping("key.yakvc.mute", InputConstants.UNKNOWN.getValue(), category)),
				KeyMappingHelper.registerKeyMapping(
						new KeyMapping("key.yakvc.deafen", InputConstants.UNKNOWN.getValue(), category)),
				KeyMappingHelper.registerKeyMapping(
						new KeyMapping("key.yakvc.open_menu", InputConstants.UNKNOWN.getValue(), category)),
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
		boolean pressed = false;
		while (debugOverlay.consumeClick()) pressed = !pressed;
		return pressed;
	}

	/** Whether the voice menu key was pressed since the last call. */
	public boolean menuPressed() {
		return pressed(openMenu);
	}

	/** Whether the Group key was pressed since the last call. Several presses in one tick count as one. */
	public boolean groupPressed() {
		return pressed(group);
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

	/** For the voice menu's buttons. */
	public void setMuted(boolean muted) {
		this.muted = muted;
	}

	public void setDeafened(boolean deafened) {
		this.deafened = deafened;
	}
}
