package io.github.desocketed.yakvc.ui;

/**
 * Coalesces settings changes so a dragged slider applies a few times a second instead of on every step: each apply can
 * reopen devices or re-encode, and the file is saved each time.
 */
final class ApplyThrottle {
	/** Two ticks, about 100 ms: fast enough to hear the change while dragging. */
	static final int TICKS_BETWEEN = 2;

	private boolean pending;
	private int ticksUntilReady;

	/** A setting changed and should be applied soon. */
	void changed() {
		pending = true;
	}

	/** Called once a tick; true when the pending change should be applied now. */
	boolean tick() {
		if (ticksUntilReady > 0) ticksUntilReady--;
		if (!pending || ticksUntilReady > 0) return false;
		pending = false;
		ticksUntilReady = TICKS_BETWEEN;
		return true;
	}

	/** Whether a change is still waiting, which closing the screen applies at once. Clears it. */
	boolean takePending() {
		boolean was = pending;
		pending = false;
		return was;
	}
}
