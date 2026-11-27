package io.github.desocketed.yakvc.heads;

/**
 * How a talking player's head moves with their voice, one {@link HeadStyle} per implementation. Pure and free of
 * allocation per frame: the caller feeds it the engine's speech features as they arrive, advances it by the frame time
 * and reads angles in radians to add to the head's, at strength 1.
 */
public sealed interface HeadMotion permits Nod, Bobble, Prosody, Jaw, Wobble {
	/** Takes one analysis of the voice: its level in dBFS and its pitch in Hz, 0 when unvoiced. */
	void features(double levelDb, double pitchHz);

	/** Moves on by {@code dt} seconds. */
	void advance(double dt);

	/** Whether it has come to rest, so it can be forgotten. */
	boolean resting();

	/** Downward tilt to add to the head's {@code xRot}. */
	float xRot();

	float yRot();

	float zRot();

	/** A motion in {@code style}; {@code phase} keeps different players apart, and any number works. */
	static HeadMotion of(HeadStyle style, double phase) {
		return switch (style) {
			case NOD -> new Nod(phase);
			case BOBBLE -> new Bobble(phase);
			case PROSODY -> new Prosody(phase);
			case JAW -> new Jaw();
			case WOBBLE -> new Wobble(phase);
		};
	}
}
