package io.github.desocketed.yakvc.ui;

import java.util.HashMap;
import java.util.Locale;
import java.util.Map;
import net.minecraft.network.chat.Component;

/**
 * Shows the engine's {@code Error} events, such as a microphone that can't open, as toasts, one after another. The
 * engine reports each device outage once, but some problems keep coming back (a sign-in that fails at every retry, an
 * encoder that fails every frame), so the same message shows again only after a few minutes; otherwise they would
 * pile up in the toast queue.
 */
public final class EngineErrorToasts {
	private static final long REPEAT_NANOS = 5 * 60 * 1_000_000_000L;

	/** When each message was last shown, within the last few minutes. */
	private final Map<String, Long> shown = new HashMap<>();

	/** Must run on the client thread. */
	public void show(String message) {
		if (!due(message, System.nanoTime())) return;
		// The engine's messages are sentence fragments, like "cannot open the microphone: ...".
		String sentence = message.isEmpty() ? message : message.substring(0, 1).toUpperCase(Locale.ROOT)
				+ message.substring(1);
		VoiceToasts.queueEngineError(Component.literal(sentence));
	}

	/** Whether to show {@code message} now, and if so remembers it. */
	boolean due(String message, long nowNanos) {
		shown.values().removeIf(at -> nowNanos - at >= REPEAT_NANOS);
		if (shown.containsKey(message)) return false;
		shown.put(message, nowNanos);
		return true;
	}
}
