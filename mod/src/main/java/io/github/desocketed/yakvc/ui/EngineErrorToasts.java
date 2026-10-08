package io.github.desocketed.yakvc.ui;

import java.util.Locale;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;

/**
 * Shows the engine's {@code Error} events, such as a microphone that can't open, as toasts. The engine reports each
 * device outage once, but some problems keep coming back (a sign-in that fails at every retry, an encoder that fails
 * every frame), so the same message shows again only after a few minutes.
 */
public final class EngineErrorToasts {
	private static final long REPEAT_NANOS = 5 * 60 * 1_000_000_000L;

	private @Nullable String last;
	private long lastNanos;

	/** Must run on the client thread. */
	public void show(String message) {
		if (!due(message, System.nanoTime())) return;
		// The engine's messages are sentence fragments, like "cannot open the microphone: ...".
		String sentence = message.isEmpty() ? message : message.substring(0, 1).toUpperCase(Locale.ROOT)
				+ message.substring(1);
		VoiceToasts.show(Component.literal(sentence));
	}

	/** Whether to show {@code message} now, and if so remembers it. */
	boolean due(String message, long nowNanos) {
		if (message.equals(last) && nowNanos - lastNanos < REPEAT_NANOS) return false;
		last = message;
		lastNanos = nowNanos;
		return true;
	}
}
