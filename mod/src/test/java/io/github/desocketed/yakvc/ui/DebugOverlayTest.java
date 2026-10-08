package io.github.desocketed.yakvc.ui;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

class DebugOverlayTest {
	private static final long ISSUED = 1_800_000_000L;
	/** Tickets last a day. */
	private static final long EXPIRY = ISSUED + 24 * 3600;

	@Test
	void ticketTimeLeftIsInHoursUntilTheLastHour() {
		assertEquals("23 h", DebugOverlay.timeLeft(EXPIRY, ISSUED + 1));
		assertEquals("23 h", DebugOverlay.timeLeft(EXPIRY, ISSUED + 59 * 60));
		assertEquals("22 h", DebugOverlay.timeLeft(EXPIRY, ISSUED + 61 * 60));
		assertEquals("1 h", DebugOverlay.timeLeft(EXPIRY, EXPIRY - 3600));
		assertEquals("59 min", DebugOverlay.timeLeft(EXPIRY, EXPIRY - 3599));
		assertEquals("1 min", DebugOverlay.timeLeft(EXPIRY, ISSUED + (23 * 60 + 59) * 60));
		assertEquals("0 min", DebugOverlay.timeLeft(EXPIRY, EXPIRY + 10));
	}
}
