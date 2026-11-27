package io.github.desocketed.yakvc;

/**
 * How a talking player's head moves with their voice, from the engine's speech features: a nod on each stressed
 * syllable (a jump in level, larger when the pitch peaks with it) through a damped spring, a slow sway that grows with
 * loudness, and a slight raise of the head as the pitch rises above the speaker's usual. Pure: the caller feeds it
 * features as they arrive and advances it by the frame time, and reads angles in radians to add to the head's.
 */
public final class HeadMotion {
	/** Levels between these map to no movement and full movement. */
	static final double QUIET_DB = -50;
	static final double LOUD_DB = -15;
	/** The movement envelope rises and falls with these time constants, in seconds. */
	static final double ATTACK = 0.08;
	static final double RELEASE = 0.2;
	/** Without features for this long, the player has stopped talking. */
	static final double STALE = 0.15;
	/** A rise of this much over the recent level is a syllable onset. */
	static final double ONSET_DB = 6;
	/** At most one nod this often. */
	static final double REFRACTORY = 0.15;
	/** The nod spring: its frequency (Hz) and damping ratio. */
	static final double NOD_HZ = 3.5;
	static final double NOD_DAMPING = 0.45;
	/** Spring speed a full-strength onset adds, in radians per second: a nod of up to about 5 degrees. */
	static final double NOD_KICK = 3;
	/** At most this much sway (yaw, roll) and pitch raise, in radians. */
	static final double SWAY = 0.05;
	static final double PITCH_RAISE = 0.08;
	/** The pitch raise is full this many semitones above the speaker's usual pitch. */
	static final double RAISE_SEMITONES = 6;

	private final double phase;
	private double envelope;
	private double target;
	private double sinceFeatures = Double.MAX_VALUE;
	private double slowDb = QUIET_DB;
	private double sinceNod = Double.MAX_VALUE;
	/** The speaker's usual pitch, as a log, and how far above it the latest is, in semitones. */
	private double usualPitch = Double.NaN;
	private double semitones;
	private double raise;
	private double nod;
	private double nodSpeed;
	private double time;

	/** {@code phase} keeps different players' sway apart; any number works. */
	public HeadMotion(double phase) {
		this.phase = phase;
	}

	/** Takes one analysis of the voice: its level in dBFS and its pitch in Hz, 0 when unvoiced. */
	public void features(double levelDb, double pitchHz) {
		sinceFeatures = 0;
		target = Math.clamp((levelDb - QUIET_DB) / (LOUD_DB - QUIET_DB), 0, 1);
		boolean voiced = pitchHz > 0;
		double accent = 0;
		if (voiced) {
			double log = Math.log(pitchHz);
			if (Double.isNaN(usualPitch)) usualPitch = log;
			semitones = 12 / Math.log(2) * (log - usualPitch);
			usualPitch += 0.05 * (log - usualPitch);
			accent = Math.clamp(semitones / RAISE_SEMITONES, 0, 1);
		}
		double rise = levelDb - slowDb;
		if (rise > ONSET_DB && sinceNod > REFRACTORY && target > 0) {
			double strength = Math.min(rise / (2 * ONSET_DB), 1) * (0.6 + 0.4 * accent);
			nodSpeed += NOD_KICK * strength;
			sinceNod = 0;
		}
		slowDb += 0.3 * (Math.max(levelDb, QUIET_DB) - slowDb);
	}

	/** Moves on by {@code dt} seconds. */
	public void advance(double dt) {
		if (!(dt > 0)) return;
		dt = Math.min(dt, 0.1);
		time += dt;
		sinceFeatures += dt;
		sinceNod += dt;
		double goal = sinceFeatures > STALE ? 0 : target;
		envelope += (goal - envelope) * (1 - Math.exp(-dt / (goal > envelope ? ATTACK : RELEASE)));
		double raiseGoal = Math.clamp(semitones / RAISE_SEMITONES, -1, 1) * envelope;
		raise += (raiseGoal - raise) * (1 - Math.exp(-dt / RELEASE));
		// Semi-implicit Euler stays stable at any frame rate up to the 0.1 s cap.
		double omega = 2 * Math.PI * NOD_HZ;
		nodSpeed += (-omega * omega * nod - 2 * NOD_DAMPING * omega * nodSpeed) * dt;
		nod += nodSpeed * dt;
	}

	/** Whether it has come to rest, so it can be forgotten. */
	public boolean resting() {
		return sinceFeatures > STALE && envelope < 1e-3 && Math.abs(nod) < 1e-3 && Math.abs(nodSpeed) < 1e-2;
	}

	/** How much the voice moves the head now, 0 to 1. */
	double envelope() {
		return envelope;
	}

	/** Downward tilt to add to the head's {@code xRot}: the nod, less the pitch raise. */
	public float xRot() {
		return (float) (nod - PITCH_RAISE * raise);
	}

	public float yRot() {
		return (float) (SWAY * envelope * noise(0.31, 0.83));
	}

	public float zRot() {
		return (float) (0.6 * SWAY * envelope * noise(0.43, 0.67));
	}

	/** Two sines at unrelated frequencies: a smooth wander in -1..1 that doesn't repeat for a long while. */
	private double noise(double hz, double otherHz) {
		double t = 2 * Math.PI * time;
		return 0.6 * Math.sin(hz * t + phase) + 0.4 * Math.sin(otherHz * t + 2.7 * phase);
	}
}
