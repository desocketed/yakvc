package io.github.desocketed.yakvc.heads;

import java.util.HashMap;
import java.util.Map;
import java.util.UUID;
import org.jspecify.annotations.Nullable;

/** The {@link HeadMotion} of every player whose voice moves their head, for the player model to read each frame. */
public final class SpeakingHeads {
	private final Map<UUID, Head> heads = new HashMap<>();
	private HeadStyle style = HeadStyle.NOD;

	private static final class Head {
		final HeadMotion motion;
		long nanos;

		Head(UUID uuid, HeadStyle style, long nanos) {
			motion = HeadMotion.of(style, uuid.hashCode() % 1000 / 100.0);
			this.nanos = nanos;
		}
	}

	/** Moves heads in {@code style} from now on; a change starts every head afresh. */
	public void style(HeadStyle style) {
		if (style == this.style) return;
		this.style = style;
		heads.clear();
	}

	/** Takes a speech event for {@code uuid}; {@code pitchHz} is 0 when unvoiced. */
	public void speech(UUID uuid, float levelDb, float pitchHz, long nanos) {
		heads.computeIfAbsent(uuid, u -> new Head(u, style, nanos)).motion.features(levelDb, pitchHz);
	}

	/** {@code uuid}'s motion advanced to {@code nanos}, or null if their voice isn't moving them. */
	public @Nullable HeadMotion at(UUID uuid, long nanos) {
		Head head = heads.get(uuid);
		if (head == null) return null;
		head.motion.advance((nanos - head.nanos) / 1e9);
		head.nanos = Math.max(head.nanos, nanos);
		return head.motion;
	}

	/** Forgets heads that have come to rest. */
	public void prune() {
		heads.values().removeIf(head -> head.motion.resting());
	}

	public void forget(UUID uuid) {
		heads.remove(uuid);
	}

	public void clear() {
		heads.clear();
	}
}
