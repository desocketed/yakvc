package io.github.desocketed.yakvc.config;

/**
 * The mod's half of the {@code [acoustics]} section: how it reads rooms and walls from the world. The engine reads
 * its own keys from the same section and ignores these.
 *
 * @param reverbMax    how much of each voice goes to the reverb when fully walled in, 0 to 1
 * @param decaySmall   seconds the reverb rings in a tight space
 * @param decayLarge   seconds it rings in a space as large as {@code roomReach}
 * @param echoLevel    how loud the distinct echo off far walls is in a big space, 0 to 1
 * @param roomReach    blocks the rays around the listener look for walls
 * @param pathBend     what going round an edge always costs, 0 to 1
 * @param pathPerBlock what each extra block of the way round costs
 */
public record SoundTuning(double reverbMax, double decaySmall, double decayLarge, double echoLevel, double roomReach,
		double pathBend, double pathPerBlock) {
	/** Big, echoey spaces where the world has them, like a mountain climbing game. */
	public static final SoundTuning DEFAULT = new SoundTuning(0.7, 0.4, 6.0, 0.6, 64, 0.4, 1 / 24.0);

	public static SoundTuning of(ClientConfig config) {
		return new SoundTuning(
				config.acoustics("reverb_max", DEFAULT.reverbMax),
				config.acoustics("decay_small", DEFAULT.decaySmall),
				config.acoustics("decay_large", DEFAULT.decayLarge),
				config.acoustics("echo_level", DEFAULT.echoLevel),
				Math.clamp(config.acoustics("room_reach", DEFAULT.roomReach), 8, 128),
				config.acoustics("path_bend", DEFAULT.pathBend),
				config.acoustics("path_per_block", DEFAULT.pathPerBlock));
	}
}
