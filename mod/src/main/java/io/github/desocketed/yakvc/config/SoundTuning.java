package io.github.desocketed.yakvc.config;

/**
 * The mod's half of the {@code acoustics} settings: how it reads rooms and walls from the world. The engine reads its
 * own keys from the same section and ignores these.
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
	public static final SoundTuning DEFAULT = of(Settings.Acoustics.DEFAULT);
	/** The most furnishing the Nether gets: see {@link #forNether}. */
	static final double NETHER_FURNISHING = 0.12;
	/** The longest the Nether rings, in seconds. */
	static final double NETHER_MAX_DECAY = 3;

	/** The mod's values, clamped to what the ray casting can take; the engine checks its own. */
	public static SoundTuning of(Settings.Acoustics acoustics) {
		return new SoundTuning(
				Math.clamp(acoustics.furnishing(), 0, 1),
				Math.clamp(acoustics.maxDecay(), 0.1, 10),
				Math.clamp(acoustics.roomReach(), 8, 128),
				Math.clamp(acoustics.echoReach(), 32, 256),
				acoustics.pathBend(),
				acoustics.pathPerBlock());
	}

	/**
	 * The tuning for the Nether, whose vast netherrack caverns should ring: furnishing at most
	 * {@value #NETHER_FURNISHING} (bare caverns hold no furniture), so rooms reverberate and echo, but decay at most
	 * {@value #NETHER_MAX_DECAY} s, so speech stays clear in the biggest of them.
	 */
	public SoundTuning forNether() {
		return new SoundTuning(Math.min(furnishing, NETHER_FURNISHING), Math.min(maxDecay, NETHER_MAX_DECAY), roomReach,
				echoReach, pathBend, pathPerBlock);
	}
}
