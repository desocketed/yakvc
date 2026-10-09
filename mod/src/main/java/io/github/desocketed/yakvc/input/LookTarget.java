package io.github.desocketed.yakvc.input;

import java.util.Map;
import java.util.Optional;
import java.util.UUID;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import org.jspecify.annotations.Nullable;

/**
 * The player under the crosshair, for the Group key. Vanilla's crosshair target only reaches a few blocks and needs
 * the hitbox itself; inviting someone across a room should work too, so each hitbox is grown with its distance, which
 * makes a narrow cone around the crosshair.
 */
public final class LookTarget {
	/** About the cone's half-angle: tan(3°). */
	private static final double SLACK = 0.05;
	/** Farther than this nobody is picked: too far to be talking face to face. */
	public static final double MAX_DISTANCE = 32;

	private LookTarget() {}

	/**
	 * The player whose hitbox, grown for the cone, the view ray from {@code eye} along {@code look} (a unit vector)
	 * hits first, or null if none. {@code players} should hold only players in view.
	 */
	public static @Nullable UUID pick(Vec3 eye, Vec3 look, Map<UUID, AABB> players) {
		Vec3 end = eye.add(look.scale(MAX_DISTANCE));
		UUID best = null;
		double bestDistance = Double.MAX_VALUE;
		for (Map.Entry<UUID, AABB> player : players.entrySet()) {
			AABB box = player.getValue();
			double distance = box.getCenter().distanceTo(eye);
			if (distance > MAX_DISTANCE) continue;
			Optional<Vec3> hit = box.inflate(distance * SLACK).clip(eye, end);
			if (hit.isEmpty()) continue;
			double along = hit.get().distanceTo(eye);
			if (along < bestDistance) {
				bestDistance = along;
				best = player.getKey();
			}
		}
		return best;
	}
}
