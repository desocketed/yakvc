package io.github.desocketed.yakvc;

import io.github.desocketed.yakvc.config.SoundTuning;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.ClipContext;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;

/**
 * How the space around the listener rings: how much of each voice goes to the reverb ({@code reverb}, 0 in open air
 * up to the tuning's maximum walled in), how long the reverb lasts ({@code decay}, seconds until it fades by 60 dB),
 * and the distinct echo off the far walls ({@code echo}, its loudness, and {@code echoDelay}, seconds until it
 * returns).
 *
 * <p>Estimated from rays cast evenly in every direction. The ground under your feet doesn't make a space echo, so only
 * rays that aren't aimed steeply down count towards how walled-in it is: open ground stays dry, while a canyon, a cave
 * or a hall rings. How far those rays went says how big the space is: a big one rings longer and sends a distinct echo
 * back, later the further its walls are.
 */
public record Room(float reverb, float decay, float echo, float echoDelay) {
	private static final int RAYS = 48;
	/** Rays aimed lower than this (the y of a unit vector) only see the floor. */
	private static final double FLOOR = -0.3;
	/** Blocks per second: sound in air, taking a block as a metre. */
	private static final double SPEED_OF_SOUND = 343;
	private static final double MAX_ECHO_DELAY = 0.45;
	/** Each tick, a room moves this much of the way to a new estimate, so walking through a doorway is smooth. */
	private static final float SMOOTHING = 0.15f;
	private static final Vec3[] DIRECTIONS = directions();

	public static final Room OPEN_AIR = new Room(0, (float) SoundTuning.DEFAULT.decaySmall(), 0, 0);

	public static Room around(BlockGetter level, Vec3 eye, SoundTuning tuning) {
		int wallRays = 0;
		int walls = 0;
		double wallDistance = 0;
		for (Vec3 direction : DIRECTIONS) {
			if (direction.y <= FLOOR) continue;
			wallRays++;
			BlockHitResult hit = level.clip(new ClipContext(eye, eye.add(direction.scale(tuning.roomReach())),
					ClipContext.Block.COLLIDER, ClipContext.Fluid.NONE, CollisionContext.empty()));
			if (hit.getType() == HitResult.Type.MISS) continue;
			walls++;
			wallDistance += hit.getLocation().distanceTo(eye);
		}
		if (walls == 0) return new Room(0, (float) tuning.decaySmall(), 0, 0);
		double enclosed = (double) walls / wallRays;
		double distance = wallDistance / walls;
		double size = distance / tuning.roomReach();
		// Under about a tenth of the reach the walls are too close for an echo to stand apart from the reverb.
		double echo = tuning.echoLevel() * enclosed * Math.clamp((size - 0.1) * 2.5, 0, 1);
		return new Room(
				(float) (tuning.reverbMax() * Math.pow(enclosed, 1.5)),
				(float) (tuning.decaySmall() + (tuning.decayLarge() - tuning.decaySmall()) * size),
				(float) echo,
				(float) Math.min(MAX_ECHO_DELAY, 2 * distance / SPEED_OF_SOUND));
	}

	/** One tick's step from this room towards {@code target}. */
	public Room towards(Room target) {
		return new Room(step(reverb, target.reverb), step(decay, target.decay), step(echo, target.echo),
				step(echoDelay, target.echoDelay));
	}

	private static float step(float from, float to) {
		return from + (to - from) * SMOOTHING;
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
