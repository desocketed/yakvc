package io.github.desocketed.yakvc.ui;

/**
 * The distance at which a voice is half as loud, which is how the Sound Tuning screen shows {@code distance_rolloff}.
 * The engine fades a voice from {@code full} blocks to the voice {@code range} as the remaining share of the way to
 * the power of the rolloff, so an exponent reads as blocks: the rolloff is whatever power puts that share at one half.
 * The screen converts both ways; the setting itself stays the exponent, as the engine takes it.
 */
record HalfVolumeDistance(double full, double range) {
	/** What the engine's config validation accepts for the exponent. */
	private static final double MIN_ROLLOFF = 0.1;
	private static final double MAX_ROLLOFF = 16;

	double blocks(double rolloff) {
		return range - (range - full) * Math.pow(0.5, 1 / rolloff);
	}

	/**
	 * Clamped to what the engine accepts: at the default range, the default rolloff of 4 is already the engine's
	 * steepest, so the nearest half-volume distance the slider offers is the default's, rounded.
	 */
	double rolloff(double blocks) {
		double rolloff = Math.log(0.5) / Math.log((range - blocks) / (range - full));
		return Math.clamp(rolloff, MIN_ROLLOFF, MAX_ROLLOFF);
	}

	int minBlocks() {
		return (int) Math.round(blocks(MAX_ROLLOFF));
	}

	/** Never the range itself, where the voice is silent. */
	int maxBlocks() {
		return (int) Math.min(Math.round(blocks(MIN_ROLLOFF)), range - 1);
	}
}
