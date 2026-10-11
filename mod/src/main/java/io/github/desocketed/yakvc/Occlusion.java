package io.github.desocketed.yakvc;

import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.phys.Vec3;

/**
 * How much of the sound between two points the blocks in between block, from 0 (open air) to 1 (fully blocked). The
 * engine turns it into a quieter, muffled voice.
 *
 * <p>This is the cheapest estimate: one straight ray, counting the opaque full blocks it crosses. Better ones (block
 * materials, sound finding its way around walls) can replace it here without the engine noticing.
 */
final class Occlusion {
	/** Each opaque block lets through this much of what reaches it. */
	private static final float PER_BLOCK = 0.5f;

	private Occlusion() {
	}

	static float between(BlockGetter level, Vec3 from, Vec3 to) {
		int[] blocks = {0};
		BlockGetter.traverseBlocks(from, to, blocks, (count, pos) -> {
			if (level.getBlockState(pos).isSolidRender()) count[0]++;
			return null;
		}, count -> null);
		return 1 - (float) Math.pow(PER_BLOCK, blocks[0]);
	}
}
