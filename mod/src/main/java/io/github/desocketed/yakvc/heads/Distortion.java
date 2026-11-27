package io.github.desocketed.yakvc.heads;

import org.jspecify.annotations.Nullable;

/**
 * How a talking player's head is deformed by their voice, one {@link HeadDistort} per implementation. Like
 * {@link HeadMotion}: pure, fed the engine's speech features and advanced by the frame time. It reads the deviation
 * of each scale axis from 1, at strength 1.
 */
public sealed interface Distortion permits Squash, Puff, Jelly {
	/** Takes one analysis of the voice: its level in dBFS and its pitch in Hz, 0 when unvoiced. */
	void features(double levelDb, double pitchHz);

	/** Moves on by {@code dt} seconds. */
	void advance(double dt);

	/** Whether it has come to rest, so it can be forgotten. */
	boolean resting();

	/** How far the width is above 1. */
	float xScale();

	/** How far the height is above 1. */
	float yScale();

	/** How far the depth is above 1. */
	float zScale();

	/** A distortion in {@code distort}, or null for {@link HeadDistort#NONE}. */
	static @Nullable Distortion of(HeadDistort distort) {
		return switch (distort) {
			case NONE -> null;
			case SQUASH -> new Squash();
			case PUFF -> new Puff();
			case JELLY -> new Jelly();
		};
	}
}
