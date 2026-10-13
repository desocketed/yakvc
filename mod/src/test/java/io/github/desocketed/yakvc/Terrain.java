package io.github.desocketed.yakvc;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.world.phys.Vec3;

/**
 * A made-up world of solid blocks, for casting rays at without Minecraft: which blocks are solid is up to the caller.
 * Rays march in small steps and report the face they enter a solid block through, as the game's ray cast does.
 */
interface Terrain {
	double STONE = 0.02;

	boolean solid(int x, int y, int z);

	/** What {@link FarEchoes} would keep from rays cast from {@code eye} in each of {@code directions}. */
	default List<FarEchoes.Hit> hits(Vec3 eye, Vec3[] directions, double reach) {
		List<FarEchoes.Hit> hits = new ArrayList<>();
		for (Vec3 direction : directions) {
			Cast cast = cast(eye, direction, reach);
			if (cast != null) hits.add(new FarEchoes.Hit(direction, cast.distance, cast.incidence, STONE, cast.tall));
		}
		return hits;
	}

	/** What {@link Room} would see from {@code eye}. */
	default List<Room.Ray> rays(Vec3 eye, Vec3[] directions, double reach) {
		List<Room.Ray> rays = new ArrayList<>();
		for (Vec3 direction : directions) {
			boolean floor = direction.y <= -0.3;
			Cast cast = cast(eye, direction, reach);
			rays.add(cast == null ? Room.Ray.miss(floor)
					: new Room.Ray(floor, true, cast.distance, cast.incidence, STONE, cast.tall));
		}
		return rays;
	}

	record Cast(double distance, double incidence, boolean tall) {
	}

	private Cast cast(Vec3 eye, Vec3 direction, double reach) {
		int[] last = block(eye);
		for (double t = 0; t <= reach; t += 0.02) {
			int[] at = block(eye.add(direction.scale(t)));
			if (!solid(at[0], at[1], at[2])) {
				last = at;
				continue;
			}
			// The face entered through points back towards where the ray came from.
			int[] normal = {last[0] - at[0], last[1] - at[1], last[2] - at[2]};
			double incidence = -(direction.x * normal[0] + direction.y * normal[1] + direction.z * normal[2]);
			boolean tall = normal[1] != 0 || Room.tall(up -> solid(at[0], at[1] + up, at[2])
					&& !solid(at[0] + normal[0], at[1] + up, at[2] + normal[2]));
			return new Cast(t, incidence, tall);
		}
		return null;
	}

	private static int[] block(Vec3 point) {
		return new int[] {(int) Math.floor(point.x), (int) Math.floor(point.y), (int) Math.floor(point.z)};
	}
}
