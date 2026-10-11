package io.github.desocketed.yakvc;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.phys.Vec3;
import org.jspecify.annotations.Nullable;

/**
 * How a voice reaches the listener: straight through whatever is in the way, or around it through an opening such as
 * a doorway, whichever loses less. Going around always muffles some, and more the longer the way is; the voice then seems to come from
 * the opening, as far away as the way it took.
 *
 * <p>The way around is the shortest path through open blocks, searched breadth-first between face neighbours within a
 * box around the two players, so it is bounded however the world looks. Steps go along the axes, so a diagonal way
 * counts a little long.
 */
public record SoundPath(Vec3 heardFrom, float occlusion) {
	/** Below this the direct path is clear enough that no way around can beat it. */
	public static final float CLEAR = 0.1f;
	/** Bending round an edge muffles a voice this much however short the way: going round is never free. */
	private static final double BEND = 0.4;
	/** Each block the way round is longer than the straight line adds this much on top. */
	private static final double PER_EXTRA_BLOCK = 1 / 24.0;
	/** How far beyond the two players the search may wander. */
	private static final int MARGIN = 8;
	/** At most this many blocks are visited per search, about a 16-block cube. */
	private static final int MAX_VISITED = 4096;

	/** The voice through what is in the way or around it, whichever is clearer. */
	public static SoundPath between(BlockGetter level, Vec3 listener, Vec3 speaker) {
		float direct = Occlusion.between(level, listener, speaker);
		SoundPath around = direct < CLEAR ? null : around(level, listener, speaker);
		return around != null && around.occlusion < direct ? around : new SoundPath(speaker, direct);
	}

	/** The voice's way around whatever is in the way, or null if there is none nearby. */
	public static @Nullable SoundPath around(BlockGetter level, Vec3 listener, Vec3 speaker) {
		List<BlockPos> path = openPath(level, BlockPos.containing(listener), BlockPos.containing(speaker));
		if (path == null) return null;
		double distance = listener.distanceTo(speaker);
		double length = Math.max(path.size() - 1, distance);
		// The opening is the furthest point along the way still in plain view of the listener.
		Vec3 opening = speaker;
		for (BlockPos pos : path) {
			Vec3 centre = Vec3.atCenterOf(pos);
			if (Occlusion.between(level, listener, centre) >= CLEAR) break;
			opening = centre;
		}
		// Right next to the listener the opening gives no direction, so the speaker's own is used.
		Vec3 direction = (opening.distanceTo(listener) < 1 ? speaker : opening).subtract(listener).normalize();
		float occlusion = (float) Math.min(1, BEND + (length - distance) * PER_EXTRA_BLOCK);
		return new SoundPath(listener.add(direction.scale(length)), occlusion);
	}

	/** The shortest path of open blocks from {@code from} to {@code to}, both included, or null if none is near. */
	private static @Nullable List<BlockPos> openPath(BlockGetter level, BlockPos from, BlockPos to) {
		BlockPos min = new BlockPos(Math.min(from.getX(), to.getX()) - MARGIN, Math.min(from.getY(), to.getY()) - MARGIN,
				Math.min(from.getZ(), to.getZ()) - MARGIN);
		BlockPos max = new BlockPos(Math.max(from.getX(), to.getX()) + MARGIN, Math.max(from.getY(), to.getY()) + MARGIN,
				Math.max(from.getZ(), to.getZ()) + MARGIN);
		Map<BlockPos, BlockPos> cameFrom = new HashMap<>();
		ArrayDeque<BlockPos> queue = new ArrayDeque<>();
		cameFrom.put(from, from);
		queue.add(from);
		while (!queue.isEmpty() && cameFrom.size() < MAX_VISITED) {
			BlockPos pos = queue.poll();
			if (pos.equals(to)) return walkBack(cameFrom, from, to);
			for (Direction direction : Direction.values()) {
				BlockPos next = pos.relative(direction);
				if (cameFrom.containsKey(next) || !inside(next, min, max) || !open(level, next)) continue;
				cameFrom.put(next, pos);
				queue.add(next);
			}
		}
		return null;
	}

	private static List<BlockPos> walkBack(Map<BlockPos, BlockPos> cameFrom, BlockPos from, BlockPos to) {
		List<BlockPos> path = new ArrayList<>();
		for (BlockPos pos = to; !pos.equals(from); pos = cameFrom.get(pos)) path.add(pos);
		path.add(from);
		return path.reversed();
	}

	/**
	 * Sound passes a block whose middle is free: air, but also an open door or a slab, unlike a fence post. A closed
	 * door also has its middle free, its panel being at the edge, so anything that opens must be open.
	 */
	private static boolean open(BlockGetter level, BlockPos pos) {
		BlockState state = level.getBlockState(pos);
		if (!state.getValueOrElse(BlockStateProperties.OPEN, true)) return false;
		return state.getCollisionShape(level, pos).toAabbs().stream().noneMatch(box -> box.contains(0.5, 0.5, 0.5));
	}

	private static boolean inside(BlockPos pos, BlockPos min, BlockPos max) {
		return pos.getX() >= min.getX() && pos.getX() <= max.getX() && pos.getY() >= min.getY()
				&& pos.getY() <= max.getY() && pos.getZ() >= min.getZ() && pos.getZ() <= max.getZ();
	}
}
