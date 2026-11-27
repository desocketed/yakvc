package io.github.desocketed.yakvc.heads;

/**
 * {@link HeadStyle#PROSODY}, after the speech-driven head motion literature (Busso et al.), where head pitch follows
 * the pitch contour and nods come with energy: the head tilts up as the pitch rises over the speaker's usual and down
 * as it falls, smoothed over a quarter second; each onset nods it in proportion to the loudness through a slow, well
 * damped spring; and the yaw drifts slowly while the voice lasts.
 */
public final class Prosody implements HeadMotion {
	/** Tilt at a full pitch accent, in radians, and how slowly it follows. */
	static final double CONTOUR = 0.12;
	static final double CONTOUR_LAG = 0.25;
	static final double NOD_HZ = 2;
	static final double NOD_DAMPING = 0.7;
	static final double NOD_KICK = 2;
	/** At most this much yaw drift, in radians, and how slowly it fades in and out. */
	static final double DRIFT = 0.1;
	static final double DRIFT_IN = 0.6;
	static final double DRIFT_OUT = 0.3;

	private final double phase;
	private final Voice voice = new Voice();
	private final Spring nod = new Spring(NOD_HZ, NOD_DAMPING);
	private double contour;
	private double drift;

	Prosody(double phase) {
		this.phase = phase;
	}

	@Override
	public void features(double levelDb, double pitchHz) {
		double onset = voice.features(levelDb, pitchHz);
		nod.kick(NOD_KICK * onset * voice.level());
	}

	@Override
	public void advance(double dt) {
		dt = voice.advance(dt);
		if (dt == 0) return;
		contour = Voice.follow(contour, voice.accent() * voice.envelope(), dt, CONTOUR_LAG, CONTOUR_LAG);
		drift = Voice.follow(drift, voice.envelope(), dt, DRIFT_IN, DRIFT_OUT);
		nod.advance(dt);
	}

	@Override
	public boolean resting() {
		return voice.silent() && nod.resting() && Math.abs(contour) < 1e-3 && drift < 1e-3;
	}

	@Override
	public float xRot() {
		return (float) (nod.position() - CONTOUR * contour);
	}

	@Override
	public float yRot() {
		return (float) (DRIFT * drift * voice.wander(0.07, 0.19, phase));
	}

	@Override
	public float zRot() {
		return 0;
	}
}
