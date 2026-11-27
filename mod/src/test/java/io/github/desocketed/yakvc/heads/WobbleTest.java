package io.github.desocketed.yakvc.heads;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

class WobbleTest {
	@Test
	void noiseIsSmoothBoundedAndZeroOnTheLattice() {
		double peak = 0;
		for (double x = -20; x < 20; x += 0.01) {
			double n = Wobble.noise(x, 7);
			peak = Math.max(peak, Math.abs(n));
			assertTrue(Math.abs(Wobble.noise(x + 0.001, 7) - n) < 0.01, "jump at " + x);
		}
		assertTrue(peak > 0.5 && peak <= 1, "peak " + peak);
		assertEquals(0, Wobble.noise(3, 7), 1e-12);
	}
}
