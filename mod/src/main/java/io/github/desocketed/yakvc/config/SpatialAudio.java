package io.github.desocketed.yakvc.config;

import com.google.gson.annotations.SerializedName;
import java.util.Locale;
import org.jspecify.annotations.Nullable;

/**
 * How much of the world's acoustics the mod works out and hands the engine, from {@code spatial_audio} in
 * {@code settings.json}. Only the mod reads it: each level skips the ray casting it doesn't need and pushes silent
 * values instead.
 */
public enum SpatialAudio {
	/** Distance and left/right panning only. */
	@SerializedName("off")
	OFF,
	/** Plus walls: muffling through them and voices coming round them, through doorways. */
	@SerializedName("simple")
	SIMPLE,
	/** Plus the room's reverb and echo and far echoes off cliffs. */
	@SerializedName("full")
	FULL;

	/** As written in the settings file, and in translation keys. */
	public String key() {
		return name().toLowerCase(Locale.ROOT);
	}

	/** The level written as {@code key}, or {@link #SIMPLE}, the default, for anything else. */
	public static SpatialAudio of(@Nullable String key) {
		for (SpatialAudio level : values()) {
			if (level.key().equals(key)) return level;
		}
		return SIMPLE;
	}
}
