package io.github.desocketed.yakvc;

import net.minecraft.core.BlockPos;
import net.minecraft.tags.BlockTags;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;

/**
 * How much of the sound between two points the blocks in between block, from 0 (open air) to 1 (fully blocked). The
 * engine turns it into a quieter, muffled voice.
 *
 * <p>One straight ray: each block whose shape it actually crosses lets through part of what reaches it, depending on
 * what the block is made of. An open door or a slab the ray passes over doesn't count. Better estimates, such as sound
 * finding its way around walls, can replace this without the engine noticing.
 */
public final class Occlusion {
	/** What an ordinary solid block (stone, wood, dirt) lets through. */
	private static final float SOLID = 0.5f;
	/** Wool and carpet soak sound up. */
	private static final float WOOL = 0.15f;
	private static final float GLASS = 0.7f;
	private static final float LEAVES = 0.85f;
	/** Per block of water or lava. */
	private static final float FLUID = 0.8f;

	private Occlusion() {
	}

	public static float between(BlockGetter level, Vec3 from, Vec3 to) {
		float[] through = {1};
		BlockGetter.traverseBlocks(from, to, through, (passing, pos) -> {
			passing[0] *= transmission(level, pos, from, to);
			// Nothing audible gets further: stop walking the ray.
			return passing[0] < 0.01f ? passing : null;
		}, passing -> passing);
		return 1 - through[0];
	}

	/** The share of sound the block at {@code pos} lets through along the segment {@code from}–{@code to}. */
	private static float transmission(BlockGetter level, BlockPos pos, Vec3 from, Vec3 to) {
		BlockState state = level.getBlockState(pos);
		float fluid = state.getFluidState().isEmpty() ? 1 : FLUID;
		if (state.isAir() || state.getCollisionShape(level, pos).clip(from, to, pos) == null) return fluid;
		SoundType sound = state.getSoundType();
		if (sound == SoundType.WOOL) return WOOL;
		if (sound == SoundType.GLASS) return GLASS;
		if (state.is(BlockTags.LEAVES)) return LEAVES;
		return SOLID * fluid;
	}
}
