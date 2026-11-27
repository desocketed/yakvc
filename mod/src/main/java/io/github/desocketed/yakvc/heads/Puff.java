package io.github.desocketed.yakvc.heads;

/** {@link HeadDistort#PUFF}: the head grows evenly with the voice's smoothed loudness and shrinks back in silence. */
public final class Puff implements Distortion {
	/** The growth at full loudness. */
	static final double GROWTH = 0.25;

	private final Voice voice = new Voice();

	Puff() {}

	@Override
	public void features(double levelDb, double pitchHz) {
		voice.features(levelDb, pitchHz);
	}

	@Override
	public void advance(double dt) {
		voice.advance(dt);
	}

	@Override
	public boolean resting() {
		return voice.silent();
	}

	@Override
	public float xScale() {
		return (float) (GROWTH * voice.envelope());
	}

	@Override
	public float yScale() {
		return xScale();
	}

	@Override
	public float zScale() {
		return xScale();
	}
}
