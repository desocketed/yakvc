package io.github.desocketed.yakvc.ui;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.toasts.SystemToast;
import net.minecraft.network.chat.Component;

/** Tells the player that voice stopped working, which otherwise would only be in the log. */
public final class VoiceToasts {
	/** One toast slot for all of them, shown for 10 s; a newer problem replaces an older one. */
	private static final SystemToast.SystemToastId ID = new SystemToast.SystemToastId(10_000L);
	/**
	 * The engine's errors, also shown for 10 s. They have their own id because they queue up rather than share a
	 * slot, and {@link #show} would otherwise replace one of them.
	 */
	public static final SystemToast.SystemToastId ENGINE_ERROR = new SystemToast.SystemToastId(10_000L);

	private VoiceToasts() {}

	/** Must run on the client thread, after the game has started. */
	public static void show(Component message) {
		SystemToast.addOrUpdate(Minecraft.getInstance().gui.toastManager(), ID,
				Component.translatable("yakvc.toast.title"), message);
	}

	/**
	 * Adds an engine error toast after any already showing, since two errors in a row (a microphone and the speakers
	 * that can't open) are different problems the player should see both of. Must run on the client thread.
	 */
	public static void queueEngineError(Component message) {
		SystemToast.add(Minecraft.getInstance().gui.toastManager(), ENGINE_ERROR,
				Component.translatable("yakvc.toast.title"), message);
	}
}
