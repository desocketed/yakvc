package io.github.desocketed.yakvc;

import io.github.desocketed.yakvc.config.SoundTuning;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.core.Direction;
import net.minecraft.tags.BlockTags;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.ClipContext;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;

/**
 * How the space around the listener rings, measured like a real room, taking a block as a metre: how walled in it is
 * ({@code reverb}, 0 in open air up to 1 shut in), its critical distance ({@code criticalDistance}, metres at which a
 * voice's reverb is as loud as the voice), how long the reverb lasts ({@code decay}, seconds until it fades by 60 dB)
 * and the echo off walls a little way off ({@code echo}, its loudness, and {@code echoDelay}, seconds until it returns;
 * walls further than {@link FarEchoes#MIN_DISTANCE} each echo apart, in {@link FarEchoes}).
 *
 * <p>Rays cast evenly in every direction measure the room: each stands for an equal cone of it, so together they give
 * its volume, its surface and how much that surface absorbs. From those, Eyring's formula gives the decay. The ground
 * under your feet doesn't make a space ring, so only rays that aren't aimed steeply down count towards how walled in
 * it is: open ground stays dry, while a canyon, a cave or a hall rings.
 */
public record Room(float reverb, float criticalDistance, float decay, float echo, float echoDelay) {
	private static final int RAYS = 48;
	/** Rays aimed lower than this (the y of a unit vector) only see the floor. */
	private static final double FLOOR = -0.3;
	/** Metres per second: sound in air. */
	private static final double SPEED_OF_SOUND = 343;
	/** The engine's echo line holds half a second. */
	private static final double MAX_ECHO_DELAY = 0.45;
	/** The shortest round trip the ear hears as an echo apart from the voice (about 50 ms): walls 8.5 m away. */
	private static final double ECHO_MIN_DISTANCE = 8.5;
	/** How loud an echo off a hard wall just far enough to stand apart is. */
	private static final double ECHO_LEVEL = 0.5;
	/** A voice is louder in front of the talker than behind; Q = 2 widens the critical distance by √2. */
	private static final double VOICE_DIRECTIVITY = 2;
	/** Air absorbs sound too, m in Sabine's 4mV: per metre, around 1 kHz at 50 % humidity. It matters only in big spaces. */
	private static final double AIR_ABSORPTION = 0.0025;
	/** A ray glancing along a wall would count a huge surface; this caps it at four times its cone's cross-section. */
	private static final double MIN_INCIDENCE = 0.25;
	/** Surfaces met within 60° of head-on face the listener and send an echo back. */
	private static final double FACING = 0.5;
	/** Each tick, a room moves this much of the way to a new estimate, so walking through a doorway is smooth. */
	private static final float SMOOTHING = 0.15f;
	private static final Vec3[] DIRECTIONS = directions(RAYS);

	public static final Room OPEN_AIR = new Room(0, 1, 0.1f, 0, 0);

	/**
	 * What one ray found.
	 *
	 * @param floor      whether it was aimed steeply down, at the floor
	 * @param hit        whether it found a surface within reach; if not, the rest are ignored
	 * @param distance   metres to the surface
	 * @param incidence  the cosine of the angle between the ray and the surface's normal
	 * @param absorption the share of sound the surface absorbs, from 0 to 1
	 */
	public record Ray(boolean floor, boolean hit, double distance, double incidence, double absorption) {
		static Ray miss(boolean floor) {
			return new Ray(floor, false, 0, 1, 1);
		}
	}

	public static Room around(BlockGetter level, Vec3 eye, SoundTuning tuning) {
		List<Ray> rays = new ArrayList<>(RAYS);
		for (Vec3 direction : DIRECTIONS) {
			boolean floor = direction.y <= FLOOR;
			BlockHitResult hit = level.clip(new ClipContext(eye, eye.add(direction.scale(tuning.roomReach())),
					ClipContext.Block.COLLIDER, ClipContext.Fluid.NONE, CollisionContext.empty()));
			if (hit.getType() == HitResult.Type.MISS) {
				rays.add(Ray.miss(floor));
				continue;
			}
			Direction face = hit.getDirection();
			double incidence = Math.abs(direction.x * face.getStepX() + direction.y * face.getStepY()
					+ direction.z * face.getStepZ());
			rays.add(new Ray(floor, true, hit.getLocation().distanceTo(eye), incidence,
					absorption(level.getBlockState(hit.getBlockPos()))));
		}
		return of(rays, tuning);
	}

	/**
	 * The room that rays cast evenly in every direction describe. Each ray stands for a cone of solid angle 4π/N: its
	 * share of the volume is r³/3 of that, and of the surface r² of that, more where it meets the surface at a slant.
	 * A ray that finds nothing within reach looks into open air: a surface at the reach that absorbs everything.
	 */
	public static Room of(List<Ray> rays, SoundTuning tuning) {
		double cone = 4 * Math.PI / rays.size();
		double volume = 0;
		double surface = 0;
		double absorbing = 0;
		int wallRays = 0;
		int walls = 0;
		double wallDistance = 0;
		int facing = 0;
		double facingDistance = 0;
		double facingAbsorption = 0;
		for (Ray ray : rays) {
			double r = ray.hit ? ray.distance : tuning.roomReach();
			double absorption = ray.hit ? Math.min(1, ray.absorption + tuning.furnishing()) : 1;
			double area = cone * r * r / (ray.hit ? Math.max(MIN_INCIDENCE, ray.incidence) : 1);
			volume += cone * r * r * r / 3;
			surface += area;
			absorbing += area * absorption;
			if (ray.floor) continue;
			wallRays++;
			if (!ray.hit) continue;
			walls++;
			wallDistance += ray.distance;
			// Further walls echo apart, each from its own direction: FarEchoes.
			if (ray.incidence < FACING || ray.distance > FarEchoes.MIN_DISTANCE) continue;
			facing++;
			facingDistance += ray.distance;
			facingAbsorption += absorption;
		}
		if (walls == 0) return OPEN_AIR;

		// Eyring: T60 = 0.161 V / (-S ln(1 - α) + 4mV), with air absorption. It matches Sabine's 0.161 V / (S α) in
		// live rooms and, unlike Sabine, stays right in dead ones. α stays below 1 so open air gives a short decay
		// rather than none.
		double alpha = Math.min(0.95, absorbing / surface);
		double eyring = 0.161 * volume / (-surface * Math.log(1 - alpha) + 4 * AIR_ABSORPTION * volume);
		double decay = Math.clamp(eyring, 0.1, tuning.maxDecay());
		// The critical distance, r_c = 0.057 √(Q V / T60), where the reverb is as loud as the voice.
		double critical = 0.057 * Math.sqrt(VOICE_DIRECTIVITY * volume / decay);

		double enclosed = (double) walls / wallRays;
		double distance = wallDistance / walls;
		// An echo stands apart from the voice only once its round trip takes about 50 ms. It is quieter the further
		// it travels (1/r) and the more the walls absorb, and fades in over a few metres so it doesn't switch on.
		// Only surfaces that face the listener send it back, so the ground stretching away doesn't.
		double echo = 0;
		if (facing > 0) {
			distance = facingDistance / facing;
			double reflected = Math.sqrt(1 - facingAbsorption / facing);
			echo = ECHO_LEVEL * facing / wallRays * reflected * Math.min(1, ECHO_MIN_DISTANCE / distance)
					* Math.clamp((distance - ECHO_MIN_DISTANCE) / 4, 0, 1);
		}
		return new Room((float) Math.pow(enclosed, 1.5), (float) critical, (float) decay, (float) echo,
				(float) Math.min(MAX_ECHO_DELAY, 2 * distance / SPEED_OF_SOUND));
	}

	/**
	 * The share of sound a block's surface absorbs, roughly the published coefficients around 1 kHz. The block's sound
	 * is a good stand-in for its material. Rays pass through water, and ground under water is as hard as water's
	 * surface, so water is left to the block beneath it.
	 */
	static double absorption(BlockState state) {
		if (state.is(BlockTags.LEAVES)) return 0.6;
		if (state.is(BlockTags.WOOL) || state.is(BlockTags.WOOL_CARPETS)) return 0.5;
		SoundType sound = state.getSoundType();
		if (sound == SoundType.WOOL || sound == SoundType.SNOW || sound == SoundType.POWDER_SNOW) return 0.5;
		if (sound == SoundType.GLASS) return 0.03;
		if (sound == SoundType.WOOD || sound == SoundType.NETHER_WOOD || sound == SoundType.BAMBOO_WOOD
				|| sound == SoundType.CHERRY_WOOD) return 0.1;
		if (sound == SoundType.GRASS || sound == SoundType.GRAVEL || sound == SoundType.SAND
				|| sound == SoundType.ROOTED_DIRT || sound == SoundType.MUD) return 0.15;
		if (sound == SoundType.STONE || sound == SoundType.DEEPSLATE || sound == SoundType.DEEPSLATE_BRICKS
				|| sound == SoundType.POLISHED_DEEPSLATE || sound == SoundType.TUFF || sound == SoundType.NETHER_BRICKS
				|| sound == SoundType.CALCITE) return 0.02;
		return 0.05;
	}

	/** One tick's step from this room towards {@code target}. */
	public Room towards(Room target) {
		return new Room(step(reverb, target.reverb), step(criticalDistance, target.criticalDistance),
				step(decay, target.decay), step(echo, target.echo), step(echoDelay, target.echoDelay));
	}

	private static float step(float from, float to) {
		return from + (to - from) * SMOOTHING;
	}

	/** Evenly spread unit vectors: a Fibonacci sphere. */
	static Vec3[] directions(int count) {
		Vec3[] directions = new Vec3[count];
		double golden = Math.PI * (3 - Math.sqrt(5));
		for (int i = 0; i < count; i++) {
			double y = 1 - 2 * (i + 0.5) / count;
			double radius = Math.sqrt(1 - y * y);
			directions[i] = new Vec3(Math.cos(golden * i) * radius, y, Math.sin(golden * i) * radius);
		}
		return directions;
	}
}
