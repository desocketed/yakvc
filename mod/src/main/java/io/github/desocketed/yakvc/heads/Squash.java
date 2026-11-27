package io.github.desocketed.yakvc.heads;

/**
 * {@link HeadDistort#SQUASH}: each syllable onset kicks a spring that stretches the head tall and thin, then squashes
 * it short and wide, keeping its volume.
 */
public final class Squash implements Distortion {
	/** The stretch at a full syllable. */
	static final double STRETCH = 0.2;
	static final double HZ = 5;
	static final double DAMPING = 0.35;
	/** The speed a full onset gives the spring. */
	static final double KICK = 8;

	private final Voice voice = new Voice();
	private final Spring spring = new Spring(HZ, DAMPING);

	Squash() {}

	@Override
	public void features(double levelDb, double pitchHz) {
		spring.kick(KICK * voice.features(levelDb, pitchHz));
	}

	@Override
	public void advance(double dt) {
		dt = voice.advance(dt);
		if (dt > 0) spring.advance(dt);
	}

	@Override
	public boolean resting() {
		return voice.silent() && spring.resting();
	}

	@Override
	public float xScale() {
		return (float) (-0.5 * STRETCH * spring.position());
	}

	@Override
	public float yScale() {
		return (float) (STRETCH * spring.position());
	}

	@Override
	public float zScale() {
		return xScale();
	}
}
