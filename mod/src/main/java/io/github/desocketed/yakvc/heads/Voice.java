package io.github.desocketed.yakvc.heads;

/**
 * What every {@link HeadMotion} reads from a voice: an envelope of its level, its syllable onsets and how far its
 * pitch is above the speaker's usual.
 */
final class Voice {
	/** Levels between these map to no movement and full movement. */
	static final double QUIET_DB = -50;
	static final double LOUD_DB = -15;
	/** The envelope rises and falls with these time constants, in seconds. */
	static final double ATTACK = 0.08;
	static final double RELEASE = 0.2;
	/** Without features for this long, the player has stopped talking. */
	static final double STALE = 0.15;
	/** A rise of this much over the recent level is a syllable onset. */
	static final double ONSET_DB = 6;
	/** At most one onset this often. */
	static final double REFRACTORY = 0.15;
	/** The pitch accent is full this many semitones above the speaker's usual pitch. */
	static final double ACCENT_SEMITONES = 6;
	/** The longest frame taken whole; longer ones are cut, so springs stay stable. */
	static final double MAX_DT = 0.1;

	private double envelope;
	private double target;
	private double sinceFeatures = Double.MAX_VALUE;
	private double slowDb = QUIET_DB;
	private double sinceOnset = Double.MAX_VALUE;
	/** The speaker's usual pitch, as a log, and how far above it the latest is, in semitones. */
	private double usualPitch = Double.NaN;
	private double semitones;
	private double time;

	/** Takes one analysis, as {@link HeadMotion#features}; returns the strength of a syllable onset in it, 0 to 1. */
	double features(double levelDb, double pitchHz) {
		sinceFeatures = 0;
		target = Math.clamp((levelDb - QUIET_DB) / (LOUD_DB - QUIET_DB), 0, 1);
		if (pitchHz > 0) {
			double log = Math.log(pitchHz);
			if (Double.isNaN(usualPitch)) usualPitch = log;
			semitones = 12 / Math.log(2) * (log - usualPitch);
			usualPitch += 0.05 * (log - usualPitch);
		}
		double onset = 0;
		double rise = levelDb - slowDb;
		if (rise > ONSET_DB && sinceOnset > REFRACTORY && target > 0) {
			onset = Math.min(rise / (2 * ONSET_DB), 1) * (0.6 + 0.4 * Math.max(accent(), 0));
			sinceOnset = 0;
		}
		slowDb += 0.3 * (Math.max(levelDb, QUIET_DB) - slowDb);
		return onset;
	}

	/** Moves on by {@code dt} seconds; returns the step taken, 0 for none. */
	double advance(double dt) {
		if (!(dt > 0)) return 0;
		dt = Math.min(dt, MAX_DT);
		time += dt;
		sinceFeatures += dt;
		sinceOnset += dt;
		envelope = follow(envelope, level(), dt, ATTACK, RELEASE);
		return dt;
	}

	/** {@code value} moved towards {@code goal} over {@code dt} with one time constant up and another down. */
	static double follow(double value, double goal, double dt, double up, double down) {
		return value + (goal - value) * (1 - Math.exp(-dt / (goal > value ? up : down)));
	}

	/** The latest level, 0 to 1, or 0 once the features stop. */
	double level() {
		return sinceFeatures > STALE ? 0 : target;
	}

	/** How much the voice moves the head now, 0 to 1. */
	double envelope() {
		return envelope;
	}

	/** How far the pitch is above the speaker's usual, -1 to 1 at {@link #ACCENT_SEMITONES}. */
	double accent() {
		return Math.clamp(semitones / ACCENT_SEMITONES, -1, 1);
	}

	double time() {
		return time;
	}

	boolean silent() {
		return sinceFeatures > STALE && envelope < 1e-3;
	}

	/** Two sines at unrelated frequencies: a smooth wander in -1..1 that doesn't repeat for a long while. */
	double wander(double hz, double otherHz, double phase) {
		double t = 2 * Math.PI * time;
		return 0.6 * Math.sin(hz * t + phase) + 0.4 * Math.sin(otherHz * t + 2.7 * phase);
	}
}
