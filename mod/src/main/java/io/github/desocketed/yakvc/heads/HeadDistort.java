package io.github.desocketed.yakvc.heads;

import com.google.gson.annotations.SerializedName;
import java.util.Locale;
import org.jspecify.annotations.Nullable;

/** The ways a voice can deform a head, from {@code speaking_heads_distort}; it combines with any {@link HeadStyle}. */
public enum HeadDistort {
	/** No deformation. */
	@SerializedName("none")
	NONE,
	/** Squash and stretch on each syllable. */
	@SerializedName("squash")
	SQUASH,
	/** The head swells with the voice's loudness. */
	@SerializedName("puff")
	PUFF,
	/** A wobbly, springy scale, different along each axis. */
	@SerializedName("jelly")
	JELLY;

	/** As written in the settings file, in commands and in translation keys. */
	public String key() {
		return name().toLowerCase(Locale.ROOT);
	}

	/** The distortion written as {@code key}, or null for anything else. */
	public static @Nullable HeadDistort of(String key) {
		for (HeadDistort distort : values()) {
			if (distort.key().equals(key)) return distort;
		}
		return null;
	}
}
