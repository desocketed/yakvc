package io.github.desocketed.yakvc.heads;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.EnumSource;

/** What every distortion must do, whatever its look. */
class DistortionTest {
	private static final double FRAME = 1 / 60.0;
	/** No distortion deviates a scale further than this at strength 1. */
	private static final double BOUND = 0.5;

	/** Talks {@code seconds} as in {@link HeadStylesTest}; returns the largest deviation of any axis. */
	private static double talk(Distortion distortion, double seconds, double loudDb) {
		double untilFeatures = 0, t = 0, peak = 0;
		for (int i = 0; i < Math.round(seconds / FRAME); i++, t += FRAME) {
			untilFeatures -= FRAME;
			if (untilFeatures <= 0) {
				boolean syllable = (t * 4) % 1 < 0.5;
				distortion.features(syllable ? loudDb : loudDb - 20, syllable ? 140 : 0);
				untilFeatures += 0.04;
			}
			distortion.advance(FRAME);
			peak = Math.max(peak, Math.max(Math.abs(distortion.xScale()),
					Math.max(Math.abs(distortion.yScale()), Math.abs(distortion.zScale()))));
		}
		return peak;
	}

	private static Distortion of(HeadDistort distort) {
		Distortion distortion = Distortion.of(distort);
		assertTrue(distortion != null);
		return distortion;
	}

	@Test
	void noneHasNoDistortion() {
		assertNull(Distortion.of(HeadDistort.NONE));
	}

	@ParameterizedTest
	@EnumSource(value = HeadDistort.class, mode = EnumSource.Mode.EXCLUDE, names = "NONE")
	void silenceStaysStill(HeadDistort distort) {
		Distortion distortion = of(distort);
		assertEquals(0, talk(distortion, 2, -100), 1e-6);
		for (int i = 0; i < 12; i++) distortion.advance(FRAME);
		assertTrue(distortion.resting());
	}

	@ParameterizedTest
	@EnumSource(value = HeadDistort.class, mode = EnumSource.Mode.EXCLUDE, names = "NONE")
	void talkingDeformsWithinBounds(HeadDistort distort) {
		for (double loudDb : new double[] {-40, -20, 0, 20}) {
			double peak = talk(of(distort), 4, loudDb);
			assertTrue(peak > 0.005 && peak < BOUND, distort + " at " + loudDb + " dB deviates " + peak);
		}
	}

	@ParameterizedTest
	@EnumSource(value = HeadDistort.class, mode = EnumSource.Mode.EXCLUDE, names = "NONE")
	void comesToRestAfterSpeech(HeadDistort distort) {
		Distortion distortion = of(distort);
		talk(distortion, 2, -20);
		for (int i = 0; i < 240; i++) distortion.advance(FRAME);
		assertTrue(distortion.resting(), distort + " still moving");
		assertEquals(0, distortion.xScale(), 0.01);
		assertEquals(0, distortion.yScale(), 0.01);
		assertEquals(0, distortion.zScale(), 0.01);
	}

	@ParameterizedTest
	@EnumSource(value = HeadDistort.class, mode = EnumSource.Mode.EXCLUDE, names = "NONE")
	void ignoresBadFrameTimes(HeadDistort distort) {
		Distortion distortion = of(distort);
		talk(distortion, 1, -20);
		float before = distortion.yScale();
		distortion.advance(Double.NaN);
		distortion.advance(-1);
		assertEquals(before, distortion.yScale());
		distortion.advance(1e9);
		assertTrue(Math.abs(distortion.yScale()) < BOUND);
	}

	@Test
	void puffGrowsWithLoudness() {
		assertTrue(talk(of(HeadDistort.PUFF), 4, -15) > talk(of(HeadDistort.PUFF), 4, -35));
	}

	@Test
	void squashKeepsItsVolume() {
		Squash squash = new Squash();
		for (int i = 0; i < 120; i++) {
			if (i % 15 == 0) squash.features(-20, 140);
			squash.advance(FRAME);
			double volume = (1 + squash.xScale()) * (1 + squash.yScale()) * (1 + squash.zScale());
			assertEquals(1, volume, 0.02);
		}
	}

	@ParameterizedTest
	@EnumSource(HeadDistort.class)
	void keyRoundTrips(HeadDistort distort) {
		assertEquals(distort, HeadDistort.of(distort.key()));
	}
}
