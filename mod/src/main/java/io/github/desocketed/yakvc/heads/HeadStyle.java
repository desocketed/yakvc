package io.github.desocketed.yakvc.heads;

import com.google.gson.annotations.SerializedName;
import java.util.Locale;
import org.jspecify.annotations.Nullable;

/** The ways a voice can move a head, from {@code speaking_heads_style} in {@code settings.json}. */
public enum HeadStyle {
	/** A nod on each stressed syllable, a slow sway and a pitch raise. */
	@SerializedName("nod")
	NOD,
	/** A bounce as long as the voice lasts, faster and bigger when loud. */
	@SerializedName("bobble")
	BOBBLE,
	/** The pitch contour tilts the head, energy nods it, and it drifts slowly sideways. */
	@SerializedName("prosody")
	PROSODY,
	/** The head dips with every syllable, like a talking jaw. */
	@SerializedName("jaw")
	JAW,
	/** A wander in yaw and roll that grows on emphasis. */
	@SerializedName("wobble")
	WOBBLE;

	/** As written in the settings file, in commands and in translation keys. */
	public String key() {
		return name().toLowerCase(Locale.ROOT);
	}

	/** The style written as {@code key}, or null for anything else. */
	public static @Nullable HeadStyle of(String key) {
		for (HeadStyle style : values()) {
			if (style.key().equals(key)) return style;
		}
		return null;
	}
}
