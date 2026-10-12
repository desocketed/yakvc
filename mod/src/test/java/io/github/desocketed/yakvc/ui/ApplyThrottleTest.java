package io.github.desocketed.yakvc.ui;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

class ApplyThrottleTest {
	@Test
	void nothingAppliesWithoutAChange() {
		ApplyThrottle throttle = new ApplyThrottle();
		assertFalse(throttle.tick());
		assertFalse(throttle.takePending());
	}

	@Test
	void aSingleChangeAppliesOnTheNextTick() {
		ApplyThrottle throttle = new ApplyThrottle();
		throttle.changed();
		assertTrue(throttle.tick());
		assertFalse(throttle.tick());
	}

	@Test
	void aDragAppliesAtMostEveryFewTicks() {
		ApplyThrottle throttle = new ApplyThrottle();
		int applied = 0;
		for (int tick = 0; tick < 20; tick++) {
			throttle.changed();
			throttle.changed();
			if (throttle.tick()) applied++;
		}
		assertTrue(applied == 20 / ApplyThrottle.TICKS_BETWEEN, applied + " applies in 20 ticks");
		// The last value of the drag is never lost.
		throttle.changed();
		assertTrue(throttle.takePending());
		assertFalse(throttle.takePending());
	}
}
