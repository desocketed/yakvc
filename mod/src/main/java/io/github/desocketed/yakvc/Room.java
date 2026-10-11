package io.github.desocketed.yakvc;

import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.ClipContext;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;

/**
 * How the space around the listener rings: how much of each voice goes to the reverb ({@code reverb}, 0 in open air
 * up to {@link #MAX_REVERB} shut in) and how long the reverb lasts ({@code decay}, seconds until it fades by 60 dB).
 *
 * <p>Estimated from rays cast evenly in every direction: the share that hit something says how enclosed the listener
 * is, and how far they went says how big the space is.
 */
public record Room(float reverb, float decay) {
	private static final int RAYS = 32;
	/** Rays that go further than this count as open air. */
	private static final double REACH = 32;
	private static final float MAX_REVERB = 0.6f;
	/** The decay of a tight space and of one as large as {@link #REACH}. */
	private static final float SMALL_DECAY = 0.3f;
	private static final float LARGE_DECAY = 3.0f;
	/** Each tick, a room moves this much of the way to a new estimate, so walking through a doorway is smooth. */
	private static final float SMOOTHING = 0.15f;
	private static final Vec3[] DIRECTIONS = directions();

	public static final Room OPEN_AIR = new Room(0, SMALL_DECAY);

	public static Room around(BlockGetter level, Vec3 eye) {
		int hits = 0;
		double distance = 0;
		for (Vec3 direction : DIRECTIONS) {
			BlockHitResult hit = level.clip(new ClipContext(eye, eye.add(direction.scale(REACH)),
					ClipContext.Block.COLLIDER, ClipContext.Fluid.NONE, CollisionContext.empty()));
			if (hit.getType() == HitResult.Type.MISS) continue;
			hits++;
			distance += hit.getLocation().distanceTo(eye);
		}
		if (hits == 0) return OPEN_AIR;
		float enclosed = (float) hits / RAYS;
		float size = (float) (distance / hits / REACH);
		// Cubed, so standing on open ground (half the rays hit the floor) has almost no reverb.
		return new Room(MAX_REVERB * enclosed * enclosed * enclosed, SMALL_DECAY + (LARGE_DECAY - SMALL_DECAY) * size);
	}

	/** One tick's step from this room towards {@code target}. */
	public Room towards(Room target) {
		return new Room(reverb + (target.reverb - reverb) * SMOOTHING, decay + (target.decay - decay) * SMOOTHING);
	}

	/** Evenly spread unit vectors: a Fibonacci sphere. */
	private static Vec3[] directions() {
		Vec3[] directions = new Vec3[RAYS];
		double golden = Math.PI * (3 - Math.sqrt(5));
		for (int i = 0; i < RAYS; i++) {
			double y = 1 - 2 * (i + 0.5) / RAYS;
			double radius = Math.sqrt(1 - y * y);
			directions[i] = new Vec3(Math.cos(golden * i) * radius, y, Math.sin(golden * i) * radius);
		}
		return directions;
	}
}
