package io.github.desocketed.yakvc.heads;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.EnumSource;

/** What every style must do, whatever its look. */
class HeadStylesTest {
	private static final double FRAME = 1 / 60.0;
	/** No style turns a head further than this, in radians, at strength 1. */
	private static final double BOUND = 0.4;

	/**
	 * Runs {@code seconds} at 60 frames a second, feeding speech at 25 Hz: syllables four times a second between
	 * {@code loudDb} and 20 dB under it, the pitch gliding round 140 Hz. Returns every frame's angles, flattened.
	 */
	private static double[] talk(HeadMotion motion, double seconds, double loudDb) {
		int frames = (int) Math.round(seconds / FRAME);
		double[] angles = new double[3 * frames];
		double untilFeatures = 0, t = 0;
		for (int i = 0; i < frames; i++, t += FRAME) {
			untilFeatures -= FRAME;
			if (untilFeatures <= 0) {
				boolean syllable = (t * 4) % 1 < 0.5;
				motion.features(syllable ? loudDb : loudDb - 20, syllable ? 140 + 40 * Math.sin(t) : 0);
				untilFeatures += 0.04;
			}
			motion.advance(FRAME);
			angles[3 * i] = motion.xRot();
			angles[3 * i + 1] = motion.yRot();
			angles[3 * i + 2] = motion.zRot();
		}
		return angles;
	}

	private static double peak(double[] angles) {
		double peak = 0;
		for (double angle : angles) peak = Math.max(peak, Math.abs(angle));
		return peak;
	}

	@ParameterizedTest
	@EnumSource(HeadStyle.class)
	void silenceStaysStill(HeadStyle style) {
		HeadMotion motion = HeadMotion.of(style, 1);
		assertEquals(0, peak(talk(motion, 2, -100)), 1e-6);
		for (int i = 0; i < 12; i++) motion.advance(FRAME);
		assertTrue(motion.resting());
	}

	@ParameterizedTest
	@EnumSource(HeadStyle.class)
	void talkingMovesWithinBounds(HeadStyle style) {
		for (double loudDb : new double[] {-40, -20, 0, 20}) {
			double peak = peak(talk(HeadMotion.of(style, 1), 4, loudDb));
			assertTrue(peak > 0.005 && peak < BOUND, style + " at " + loudDb + " dB peaks at " + peak + " rad");
		}
	}

	@ParameterizedTest
	@EnumSource(HeadStyle.class)
	void louderMovesMore(HeadStyle style) {
		double quiet = peak(talk(HeadMotion.of(style, 1), 4, -35));
		double loud = peak(talk(HeadMotion.of(style, 1), 4, -15));
		assertTrue(loud > quiet, style + ": " + quiet + " vs " + loud);
	}

	@ParameterizedTest
	@EnumSource(HeadStyle.class)
	void comesToRestAfterSpeech(HeadStyle style) {
		HeadMotion motion = HeadMotion.of(style, 1);
		talk(motion, 2, -20);
		for (int i = 0; i < 180; i++) motion.advance(FRAME);
		assertTrue(motion.resting(), style + " still moving");
		assertEquals(0, motion.xRot(), 0.01);
		assertEquals(0, motion.yRot(), 0.01);
		assertEquals(0, motion.zRot(), 0.01);
	}

	@ParameterizedTest
	@EnumSource(HeadStyle.class)
	void deterministic(HeadStyle style) {
		assertArrayEquals(talk(HeadMotion.of(style, 3), 3, -20), talk(HeadMotion.of(style, 3), 3, -20));
	}

	@ParameterizedTest
	@EnumSource(HeadStyle.class)
	void ignoresBadFrameTimes(HeadStyle style) {
		HeadMotion motion = HeadMotion.of(style, 1);
		talk(motion, 1, -20);
		float before = motion.xRot();
		motion.advance(Double.NaN);
		motion.advance(-1);
		motion.advance(0);
		assertEquals(before, motion.xRot());
		motion.advance(1e9);
		assertTrue(Math.abs(motion.xRot()) < BOUND);
	}

	@ParameterizedTest
	@EnumSource(HeadStyle.class)
	void keyRoundTrips(HeadStyle style) {
		assertEquals(style, HeadStyle.of(style.key()));
	}
}
