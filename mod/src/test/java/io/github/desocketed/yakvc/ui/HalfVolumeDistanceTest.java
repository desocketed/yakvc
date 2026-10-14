package io.github.desocketed.yakvc.ui;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import io.github.desocketed.yakvc.config.Settings;
import org.junit.jupiter.api.Test;

class HalfVolumeDistanceTest {
	private static final HalfVolumeDistance DISTANCE = new HalfVolumeDistance(
			Settings.Acoustics.DEFAULT.fullVolumeDistance(), Settings.DEFAULT.voiceRange());

	@Test
	void blocksAndRolloffAreEachOthersInverse() {
		for (int blocks = DISTANCE.minBlocks(); blocks <= DISTANCE.maxBlocks(); blocks++) {
			assertEquals(blocks, DISTANCE.blocks(DISTANCE.rolloff(blocks)), 0.5, "round trip of " + blocks);
		}
	}

	@Test
	void aFartherHalfVolumeDistanceIsAGentlerFade() {
		int near = DISTANCE.minBlocks();
		int far = DISTANCE.maxBlocks();
		assertTrue(near < far);
		assertTrue(DISTANCE.rolloff(far) < DISTANCE.rolloff(near));
	}

	@Test
	void theDefaultFadeHalvesBetweenFullVolumeAndTheRange() {
		double blocks = DISTANCE.blocks(Settings.Acoustics.DEFAULT.distanceRolloff());
		assertTrue(blocks > DISTANCE.full() && blocks < DISTANCE.range(), blocks + " blocks");
	}
}
