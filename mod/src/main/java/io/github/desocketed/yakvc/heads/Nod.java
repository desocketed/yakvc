package io.github.desocketed.yakvc.heads;

/**
 * {@link HeadStyle#NOD}: a nod on each stressed syllable (an onset, larger when the pitch peaks with it) through a
 * damped spring, a slow sway that grows with loudness, and a slight raise of the head as the pitch rises above the
 * speaker's usual.
 */
public final class Nod implements HeadMotion {
	static final double NOD_HZ = 3.5;
	static final double NOD_DAMPING = 0.45;
	/** Spring speed a full-strength onset adds, in radians per second: a nod of up to about 5 degrees. */
	static final double NOD_KICK = 3;
	/** At most this much sway (yaw, roll) and pitch raise, in radians. */
	static final double SWAY = 0.05;
	static final double PITCH_RAISE = 0.08;

	private final double phase;
	private final Voice voice = new Voice();
	private final Spring nod = new Spring(NOD_HZ, NOD_DAMPING);
	private double raise;

	Nod(double phase) {
		this.phase = phase;
	}

	@Override
	public void features(double levelDb, double pitchHz) {
		nod.kick(NOD_KICK * voice.features(levelDb, pitchHz));
	}

	@Override
	public void advance(double dt) {
		dt = voice.advance(dt);
		if (dt == 0) return;
		raise = Voice.follow(raise, voice.accent() * voice.envelope(), dt, Voice.RELEASE, Voice.RELEASE);
		nod.advance(dt);
	}

	@Override
	public boolean resting() {
		return voice.silent() && nod.resting();
	}

	/** How much the voice moves the head now, 0 to 1. */
	double envelope() {
		return voice.envelope();
	}

	/** The nod, less the pitch raise. */
	@Override
	public float xRot() {
		return (float) (nod.position() - PITCH_RAISE * raise);
	}

	@Override
	public float yRot() {
		return (float) (SWAY * voice.envelope() * voice.wander(0.31, 0.83, phase));
	}

	@Override
	public float zRot() {
		return (float) (0.6 * SWAY * voice.envelope() * voice.wander(0.43, 0.67, phase));
	}
}
