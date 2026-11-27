package io.github.desocketed.yakvc.heads;

/**
 * {@link HeadStyle#WOBBLE}: a wander in yaw and roll from one-dimensional gradient noise (Perlin's, smooth and never
 * repeating), scaled by loudness and growing on emphasis: each onset, stronger with a pitch accent, excites it for a
 * moment.
 */
public final class Wobble implements HeadMotion {
	/** At most this much yaw and roll, in radians, when excited. */
	static final double YAW = 0.16;
	static final double ROLL = 0.12;
	/** The noise's rate, in lattice cells a second, calm and excited. */
	static final double CALM_HZ = 0.8;
	static final double EXCITED_HZ = 2.5;
	/** How much is left when calm, and how fast an onset's excitement fades, in seconds. */
	static final double CALM = 0.3;
	static final double FADE = 0.4;

	private final Voice voice = new Voice();
	private final int seed;
	private double position;
	private double excitement;

	Wobble(double phase) {
		seed = Double.hashCode(phase);
	}

	@Override
	public void features(double levelDb, double pitchHz) {
		excitement = Math.max(excitement, voice.features(levelDb, pitchHz));
	}

	@Override
	public void advance(double dt) {
		dt = voice.advance(dt);
		if (dt == 0) return;
		position += dt * (CALM_HZ + (EXCITED_HZ - CALM_HZ) * excitement);
		excitement *= Math.exp(-dt / FADE);
	}

	@Override
	public boolean resting() {
		return voice.silent();
	}

	private double amplitude() {
		return voice.envelope() * (CALM + (1 - CALM) * excitement);
	}

	@Override
	public float xRot() {
		return 0;
	}

	@Override
	public float yRot() {
		return (float) (YAW * amplitude() * noise(position, seed));
	}

	@Override
	public float zRot() {
		return (float) (ROLL * amplitude() * noise(position + 57.3, seed ^ 0x5bd1e995));
	}

	/** Perlin's gradient noise in one dimension, in -1..1: a random slope at each integer, blended by a quintic. */
	static double noise(double x, int seed) {
		double cell = Math.floor(x);
		double t = x - cell;
		double a = gradient((long) cell, seed) * t;
		double b = gradient((long) cell + 1, seed) * (t - 1);
		double fade = t * t * t * (t * (t * 6 - 15) + 10);
		// The extremes of 1D gradient noise with slopes in -1..1 are at 0.5 in magnitude.
		return Math.clamp(2 * (a + (b - a) * fade), -1, 1);
	}

	/** A slope in -1..1 hashed from {@code cell}. */
	private static double gradient(long cell, int seed) {
		long h = cell * 0x9E3779B97F4A7C15L + seed;
		h ^= h >>> 33;
		h *= 0xff51afd7ed558ccdL;
		h ^= h >>> 33;
		return (h >>> 11) * 0x1.0p-53 * 2 - 1;
	}
}
