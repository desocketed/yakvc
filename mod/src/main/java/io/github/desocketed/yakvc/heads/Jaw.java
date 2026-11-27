package io.github.desocketed.yakvc.heads;

/**
 * {@link HeadStyle#JAW}: the head dips with every syllable and comes back between them, like a talking jaw. It
 * follows the level fast, less part of its recent average, so a steady voice still opens and closes on its syllables.
 */
public final class Jaw implements HeadMotion {
	/** The dip at a full syllable, in radians. */
	static final double OPEN = 0.15;
	/** The fast follower's time constants, and the average's, in seconds. */
	static final double OPEN_TIME = 0.02;
	static final double CLOSE_TIME = 0.06;
	static final double AVERAGE_TIME = 0.3;
	/** How much of the average is taken off: the rest is a steady open while the voice lasts. */
	static final double CONTRAST = 0.7;

	private final Voice voice = new Voice();
	private double fast;
	private double average;

	Jaw() {}

	@Override
	public void features(double levelDb, double pitchHz) {
		voice.features(levelDb, pitchHz);
	}

	@Override
	public void advance(double dt) {
		dt = voice.advance(dt);
		if (dt == 0) return;
		fast = Voice.follow(fast, voice.level(), dt, OPEN_TIME, CLOSE_TIME);
		average = Voice.follow(average, voice.level(), dt, AVERAGE_TIME, AVERAGE_TIME);
	}

	@Override
	public boolean resting() {
		return voice.silent() && fast < 1e-3;
	}

	@Override
	public float xRot() {
		return (float) (OPEN * Math.max(fast - CONTRAST * average, 0));
	}

	@Override
	public float yRot() {
		return 0;
	}

	@Override
	public float zRot() {
		return 0;
	}
}
