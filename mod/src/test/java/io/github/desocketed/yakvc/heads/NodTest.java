package io.github.desocketed.yakvc.heads;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

class NodTest {
	private static final double FRAME = 1 / 60.0;

	/** Runs {@code seconds} at 60 frames a second, feeding {@code level} and {@code pitch} at 25 Hz. */
	private static double[] run(Nod motion, double seconds, double levelDb, double pitchHz) {
		double maxNod = 0, maxSway = 0;
		double untilFeatures = 0;
		for (double t = 0; t < seconds; t += FRAME) {
			untilFeatures -= FRAME;
			if (untilFeatures <= 0) {
				motion.features(levelDb, pitchHz);
				untilFeatures += 0.04;
			}
			motion.advance(FRAME);
			maxNod = Math.max(maxNod, motion.xRot());
			maxSway = Math.max(maxSway, Math.abs(motion.yRot()));
		}
		return new double[] {maxNod, maxSway};
	}

	/** Advances {@code seconds} in frames, with no features. */
	private static void idle(Nod motion, double seconds) {
		int frames = (int) Math.round(seconds / FRAME);
		for (int i = 0; i < frames; i++) motion.advance(seconds / frames);
	}

	@Test
	void silenceStaysStill() {
		Nod motion = new Nod(1);
		double[] peak = run(motion, 2, -100, 0);
		assertEquals(0, peak[0], 1e-6);
		assertEquals(0, peak[1], 1e-6);
		idle(motion, Voice.STALE + FRAME);
		assertTrue(motion.resting());
	}

	@Test
	void anOnsetNodsAndSettles() {
		Nod motion = new Nod(1);
		run(motion, 0.5, -100, 0);
		double nod = run(motion, 0.2, -20, 120)[0];
		assertTrue(nod > 0.03 && nod < 0.15, "nod of " + nod + " rad");
		// The spring rings down while the level holds, and the head comes to rest once speech stops.
		run(motion, 1, -20, 120);
		assertTrue(Math.abs(motion.xRot()) < 0.01, "still nodding " + motion.xRot());
		idle(motion, 1.5);
		assertTrue(motion.resting());
	}

	@Test
	void pitchAccentNodsHarder() {
		Nod flat = new Nod(1);
		Nod accented = new Nod(1);
		run(flat, 1, -45, 120);
		run(accented, 1, -45, 120);
		double plain = run(flat, 0.2, -20, 120)[0];
		double accent = run(accented, 0.2, -20, 200)[0];
		assertTrue(accent > plain * 1.2, plain + " vs " + accent);
	}

	@Test
	void swayGrowsWithLoudness() {
		double quiet = run(new Nod(1), 3, -42, 120)[1];
		double loud = run(new Nod(1), 3, -18, 120)[1];
		assertTrue(loud > 2 * quiet, quiet + " vs " + loud);
		assertTrue(loud <= Nod.SWAY);
	}

	@Test
	void higherPitchRaisesTheHead() {
		Nod motion = new Nod(1);
		run(motion, 2, -20, 120);
		double usual = motion.xRot();
		run(motion, 0.5, -20, 170);
		assertTrue(motion.xRot() < usual - 0.02, usual + " vs " + motion.xRot());
	}

	@Test
	void envelopeAttacksFasterThanItReleases() {
		Nod rising = new Nod(0);
		rising.features(Voice.LOUD_DB, 0);
		rising.advance(Voice.ATTACK);
		assertEquals(1 - Math.exp(-1), rising.envelope(), 0.01);
		Nod falling = new Nod(0);
		run(falling, 1, Voice.LOUD_DB, 0);
		idle(falling, Voice.STALE + FRAME);
		double start = falling.envelope();
		idle(falling, Voice.RELEASE);
		assertEquals(start * Math.exp(-1), falling.envelope(), 0.01);
	}
}
