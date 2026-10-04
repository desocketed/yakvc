package io.github.desocketed.yakvc.ui;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.toasts.SystemToast;
import net.minecraft.network.chat.Component;

/** Tells the player that voice stopped working, which otherwise would only be in the log. */
public final class VoiceToasts {
	/** One toast slot for all of them, shown for 10 s; a newer problem replaces an older one. */
	private static final SystemToast.SystemToastId ID = new SystemToast.SystemToastId(10_000L);

	private VoiceToasts() {}

	/** Must run on the client thread, after the game has started. */
	public static void show(Component message) {
		SystemToast.addOrUpdate(Minecraft.getInstance().gui.toastManager(), ID,
				Component.translatable("yakvc.toast.title"), message);
	}
}
