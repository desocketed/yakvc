package io.github.desocketed.yakvc.heads;

/**
 * {@link HeadDistort#JELLY}: each syllable onset sets a loose spring ringing along each of height and width, at
 * unrelated rates and in opposition, and the depth takes up what they give, so the head wobbles like jelly.
 */
public final class Jelly implements Distortion {
	/** The wobble at a full syllable. */
	static final double WOBBLE = 0.18;
	static final double KICK = 9;
	private final Voice voice = new Voice();
	private final Spring height = new Spring(4, 0.12);
	private final Spring width = new Spring(5.7, 0.12);

	Jelly() {}

	@Override
	public void features(double levelDb, double pitchHz) {
		double onset = voice.features(levelDb, pitchHz);
		height.kick(KICK * onset);
		width.kick(-0.7 * KICK * onset);
	}

	@Override
	public void advance(double dt) {
		dt = voice.advance(dt);
		if (dt == 0) return;
		height.advance(dt);
		width.advance(dt);
	}

	@Override
	public boolean resting() {
		return voice.silent() && height.resting() && width.resting();
	}

	@Override
	public float xScale() {
		return (float) (WOBBLE * width.position());
	}

	@Override
	public float yScale() {
		return (float) (WOBBLE * height.position());
	}

	@Override
	public float zScale() {
		return (float) (-0.5 * WOBBLE * (height.position() + width.position()));
	}
}
