package io.github.desocketed.yakvc.ui;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

class EngineErrorToastsTest {
	private static final long MINUTE = 60 * 1_000_000_000L;

	@Test
	void repeatsTheSameMessageOnlyAfterAFewMinutes() {
		EngineErrorToasts toasts = new EngineErrorToasts();
		assertTrue(toasts.due("voice encoder failed", 0));
		assertFalse(toasts.due("voice encoder failed", 1));
		assertFalse(toasts.due("voice encoder failed", 4 * MINUTE));
		assertTrue(toasts.due("voice encoder failed", 5 * MINUTE));
	}

	@Test
	void showsANewMessageRightAway() {
		EngineErrorToasts toasts = new EngineErrorToasts();
		assertTrue(toasts.due("cannot open the microphone: busy", 0));
		assertTrue(toasts.due("cannot open the speakers: busy", 1));
		assertTrue(toasts.due("cannot open the microphone: busy", 2));
	}
}
