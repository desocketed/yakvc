package io.github.desocketed.yakvc.heads;

/**
 * {@link HeadStyle#BOBBLE}: a cartoon bounce for as long as the voice lasts, down and up two to four times a second
 * as it gets louder, with a side-to-side tilt at half the rate, each onset bouncing it higher for a moment.
 */
public final class Bobble implements HeadMotion {
	/** At most this much bounce (pitch) and tilt (roll), in radians. */
	static final double BOUNCE = 0.22;
	static final double TILT = 0.12;
	/** Bounces a second when quiet and when loud. */
	static final double QUIET_HZ = 2;
	static final double LOUD_HZ = 4;
	/** How much an onset adds to the bounce, and how fast that fades, in seconds. */
	static final double ACCENT = 0.5;
	static final double ACCENT_FADE = 0.25;

	private final Voice voice = new Voice();
	/** Bounces so far; advanced by the rate, so a change of rate doesn't jump. */
	private double cycles;
	private double accent;

	Bobble(double phase) {
		cycles = phase;
	}

	@Override
	public void features(double levelDb, double pitchHz) {
		accent = Math.max(accent, voice.features(levelDb, pitchHz));
	}

	@Override
	public void advance(double dt) {
		dt = voice.advance(dt);
		if (dt == 0) return;
		cycles += dt * (QUIET_HZ + (LOUD_HZ - QUIET_HZ) * voice.level());
		cycles -= Math.floor(cycles / 2) * 2;
		accent *= Math.exp(-dt / ACCENT_FADE);
	}

	@Override
	public boolean resting() {
		return voice.silent();
	}

	private double amplitude() {
		return voice.envelope() * (1 + ACCENT * accent);
	}

	@Override
	public float xRot() {
		return (float) (BOUNCE * amplitude() * 0.5 * (1 - Math.cos(2 * Math.PI * cycles)));
	}

	@Override
	public float yRot() {
		return 0;
	}

	@Override
	public float zRot() {
		return (float) (TILT * amplitude() * Math.sin(Math.PI * cycles));
	}
}
