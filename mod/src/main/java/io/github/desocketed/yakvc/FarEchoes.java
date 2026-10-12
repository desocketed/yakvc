package io.github.desocketed.yakvc;

import io.github.desocketed.yakvc.config.SoundTuning;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.List;
import net.minecraft.core.Direction;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.ClipContext;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;

/**
 * The distinct echoes off far cliffs and walls, the "hello… hello…" of a valley that a diffuse reverb can't give.
 *
 * <p>Long rays, out to the echo reach, find the surfaces. They cost too much to cast all at once, so a few go out
 * each tick, turning through every direction about once a second. What each found is kept as a point in the world,
 * so the picture stays right while the listener moves between visits. Chunks that aren't loaded hold no blocks, so a
 * ray through them finds nothing, as if the land ended there.
 *
 * <p>Nearer surfaces belong to the {@link Room}: its single echo covers walls up to {@link #MIN_DISTANCE} away, these
 * everything beyond, so no wall echoes twice.
 */
public final class FarEchoes {
	/** At most this many far echoes play at once, as the engine has as many taps. */
	public static final int COUNT = 4;
	/** Directions the long rays turn through. */
	private static final int RAYS = 128;
	/** Rays cast each tick: every direction once in about 1.1 s. */
	private static final int RAYS_PER_TICK = 6;
	/** An echo stands apart from the voice once its round trip takes about 100 ms: walls 17 m away. */
	public static final double MIN_DISTANCE = 17;
	/** Metres per second: sound in air. */
	private static final double SPEED_OF_SOUND = 343;
	/** The engine holds an echo back for at most 1.5 s: a cliff 250 m away. */
	private static final double MAX_DELAY = 1.5;
	/** Surfaces met within 60° of head on face the listener and send an echo back. */
	private static final double FACING = 0.5;
	/** Hits this close in direction (cos 40°) and distance (25 %) are taken for one wall. */
	private static final double SAME_DIRECTION = 0.77;
	private static final double SAME_DISTANCE = 0.25;
	/** A wall that fills this many rays is broad enough to echo fully; a lone pillar echoes less. */
	private static final double BROAD_WALL_RAYS = 3;
	/**
	 * Where an echo is as loud as the {@link Room}'s, off a hard wall: 8.5 m away. Further, spherical spreading makes it
	 * quieter as 1/(2d), the way it travels.
	 */
	private static final double REFERENCE_DISTANCE = 8.5;
	private static final double REFERENCE_LEVEL = 0.5;
	/**
	 * Air absorbs highs over distance: a low-pass at this many Hz·m over the way the echo travels (plus 30 m), about
	 * 7 kHz off a wall 17 m away and 1 kHz off one 250 m away, close to the published absorption around 4 kHz.
	 */
	private static final double AIR_CUTOFF = 480_000;
	private static final Vec3[] DIRECTIONS = Room.directions(RAYS);

	/**
	 * A far echo as the engine plays it.
	 *
	 * @param direction a unit vector from the listener to the wall
	 * @param delay     seconds until it comes back
	 * @param gain      how loud it comes back
	 * @param cutoff    Hz above which the air has dulled it
	 */
	public record Echo(Vec3 direction, double delay, double gain, double cutoff) {
	}

	/**
	 * What a ray found, seen from the listener now.
	 *
	 * @param direction  a unit vector from the listener to the surface
	 * @param distance   metres to it
	 * @param incidence  the cosine of the angle between the way to it and its normal, negative if it faces away
	 * @param absorption the share of sound it absorbs, from 0 to 1
	 */
	public record Hit(Vec3 direction, double distance, double incidence, double absorption) {
	}

	/** Where a ray hit, with the face's normal: still right after the listener moves. */
	private record Surface(Vec3 point, Vec3 normal, double absorption) {
	}

	private final Surface[] surfaces = new Surface[RAYS];
	private int next;

	/** Casts this tick's rays from {@code eye}, then picks the echoes from everything found so far. */
	public List<Echo> update(BlockGetter level, Vec3 eye, SoundTuning tuning) {
		for (int i = 0; i < RAYS_PER_TICK; i++) {
			Vec3 direction = DIRECTIONS[next];
			BlockHitResult hit = level.clip(new ClipContext(eye, eye.add(direction.scale(tuning.echoReach())),
					ClipContext.Block.COLLIDER, ClipContext.Fluid.NONE, CollisionContext.empty()));
			surfaces[next] = hit.getType() == HitResult.Type.MISS ? null
					: new Surface(hit.getLocation(), normal(hit.getDirection()),
							Room.absorption(level.getBlockState(hit.getBlockPos())));
			next = (next + 1) % RAYS;
		}
		List<Hit> hits = new ArrayList<>();
		for (Surface surface : surfaces) {
			if (surface == null) continue;
			Vec3 to = surface.point.subtract(eye);
			double distance = to.length();
			// Kept from before the listener moved, it may now be out of reach.
			if (distance < 1e-6 || distance > tuning.echoReach()) continue;
			Vec3 direction = to.scale(1 / distance);
			hits.add(new Hit(direction, distance, -direction.dot(surface.normal), surface.absorption));
		}
		return pick(hits, RAYS);
	}

	private static Vec3 normal(Direction face) {
		return new Vec3(face.getStepX(), face.getStepY(), face.getStepZ());
	}

	/** Forgets every surface, for a new world. */
	public void clear() {
		Arrays.fill(surfaces, null);
	}

	/**
	 * The loudest far echoes that {@code hits}, out of {@code rays} cast evenly, describe. Hits far enough away and
	 * facing the listener are grouped into walls: hits pointing roughly the same way at about the same distance. Each
	 * wall echoes from its most head-on hit, which is nearest to where it mirrors the voice back. A wall is louder the
	 * more of the sky it fills (up to {@link #BROAD_WALL_RAYS} rays), the less it absorbs and the nearer it is.
	 */
	public static List<Echo> pick(List<Hit> hits, int rays) {
		List<Hit> candidates = new ArrayList<>();
		for (Hit hit : hits) {
			if (hit.distance >= MIN_DISTANCE && hit.incidence >= FACING) candidates.add(hit);
		}
		candidates.sort(Comparator.comparingDouble(Hit::incidence).reversed());
		List<List<Hit>> walls = new ArrayList<>();
		for (Hit hit : candidates) {
			List<Hit> wall = walls.stream().filter(w -> sameWall(w.getFirst(), hit)).findFirst().orElse(null);
			if (wall == null) {
				wall = new ArrayList<>();
				walls.add(wall);
			}
			wall.add(hit);
		}
		walls.sort(Comparator.comparingDouble((List<Hit> wall) -> strength(wall, rays)).reversed());
		return walls.stream().limit(COUNT).map(FarEchoes::echo).toList();
	}

	private static boolean sameWall(Hit a, Hit b) {
		return a.direction.dot(b.direction) >= SAME_DIRECTION
				&& Math.abs(a.distance - b.distance) <= SAME_DISTANCE * a.distance;
	}

	/** How much of the sphere the wall fills × how much it reflects / how far it is. */
	private static double strength(List<Hit> wall, int rays) {
		return (double) wall.size() / rays * reflected(wall) / wall.getFirst().distance;
	}

	/** The share of the sound's amplitude the wall sends back. */
	private static double reflected(List<Hit> wall) {
		return Math.sqrt(1 - wall.stream().mapToDouble(Hit::absorption).average().orElse(1));
	}

	private static Echo echo(List<Hit> wall) {
		Hit head = wall.getFirst();
		double path = 2 * head.distance;
		double gain = REFERENCE_LEVEL * REFERENCE_DISTANCE / head.distance * reflected(wall)
				* Math.min(1, wall.size() / BROAD_WALL_RAYS);
		return new Echo(head.direction, Math.min(MAX_DELAY, path / SPEED_OF_SOUND), gain, AIR_CUTOFF / (30 + path));
	}
}
