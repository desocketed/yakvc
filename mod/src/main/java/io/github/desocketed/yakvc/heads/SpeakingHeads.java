package io.github.desocketed.yakvc.heads;

import java.util.HashMap;
import java.util.Map;
import java.util.UUID;
import org.jspecify.annotations.Nullable;

/** The {@link HeadMotion} of every player whose voice moves their head, for the player model to read each frame. */
public final class SpeakingHeads {
	private final Map<UUID, Head> heads = new HashMap<>();
	private HeadStyle style = HeadStyle.NOD;
	private HeadDistort distort = HeadDistort.NONE;

	private static final class Head {
		final HeadMotion motion;
		final @Nullable Distortion distortion;
		long nanos;

		Head(UUID uuid, HeadStyle style, HeadDistort distort, long nanos) {
			motion = HeadMotion.of(style, uuid.hashCode() % 1000 / 100.0);
			distortion = Distortion.of(distort);
			this.nanos = nanos;
		}
	}

	/** Moves heads in {@code style} and deforms them in {@code distort} from now on; a change starts every head afresh. */
	public void style(HeadStyle style, HeadDistort distort) {
		if (style == this.style && distort == this.distort) return;
		this.style = style;
		this.distort = distort;
		heads.clear();
	}

	/** Takes a speech event for {@code uuid}; {@code pitchHz} is 0 when unvoiced. */
	public void speech(UUID uuid, float levelDb, float pitchHz, long nanos) {
		Head head = heads.computeIfAbsent(uuid, u -> new Head(u, style, distort, nanos));
		head.motion.features(levelDb, pitchHz);
		if (head.distortion != null) head.distortion.features(levelDb, pitchHz);
	}

	/** {@code uuid}'s motion advanced to {@code nanos} (and their {@link #distortion}), or null if their voice isn't moving them. */
	public @Nullable HeadMotion at(UUID uuid, long nanos) {
		Head head = heads.get(uuid);
		if (head == null) return null;
		double dt = (nanos - head.nanos) / 1e9;
		head.motion.advance(dt);
		if (head.distortion != null) head.distortion.advance(dt);
		head.nanos = Math.max(head.nanos, nanos);
		return head.motion;
	}

	/** {@code uuid}'s distortion as {@link #at} last advanced it, or null if there is none. */
	public @Nullable Distortion distortion(UUID uuid) {
		Head head = heads.get(uuid);
		return head == null ? null : head.distortion;
	}

	/** Forgets heads that have come to rest. */
	public void prune() {
		heads.values().removeIf(head -> head.motion.resting() && (head.distortion == null || head.distortion.resting()));
	}

	public void forget(UUID uuid) {
		heads.remove(uuid);
	}

	public void clear() {
		heads.clear();
	}
}
