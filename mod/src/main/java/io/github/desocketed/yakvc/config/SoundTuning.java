package io.github.desocketed.yakvc.config;

/**
 * The mod's half of the {@code [acoustics]} section: how it reads rooms and walls from the world. The engine reads
 * its own keys from the same section and ignores these.
 *
 * @param furnishing   absorption added to every surface, from 0 to 1, for what bare blocks don't show: furniture,
 *                     clutter, the players themselves. Without it a stone box rings far longer than any real room
 * @param maxDecay     seconds the longest reverb lasts, however big the space
 * @param roomReach    blocks the rays around the listener look for walls
 * @param echoReach    blocks the long rays look for far cliffs and walls that echo
 * @param pathBend     what going round an edge always costs, 0 to 1
 * @param pathPerBlock what each extra block of the way round costs
 */
public record SoundTuning(double furnishing, double maxDecay, double roomReach, double echoReach, double pathBend,
		double pathPerBlock) {
	public static final SoundTuning DEFAULT = new SoundTuning(0.12, 6.0, 64, 200, 0.4, 1 / 24.0);

	public static SoundTuning of(ClientConfig config) {
		return new SoundTuning(
				Math.clamp(config.acoustics("furnishing", DEFAULT.furnishing), 0, 1),
				Math.clamp(config.acoustics("max_decay", DEFAULT.maxDecay), 0.1, 10),
				Math.clamp(config.acoustics("room_reach", DEFAULT.roomReach), 8, 128),
				Math.clamp(config.acoustics("echo_reach", DEFAULT.echoReach), 32, 256),
				config.acoustics("path_bend", DEFAULT.pathBend),
				config.acoustics("path_per_block", DEFAULT.pathPerBlock));
	}
}
