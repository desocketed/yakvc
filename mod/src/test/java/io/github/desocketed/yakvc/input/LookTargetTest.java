package io.github.desocketed.yakvc.input;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;

import java.util.Map;
import java.util.UUID;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import org.junit.jupiter.api.Test;

class LookTargetTest {
	private static final UUID NEAR = UUID.fromString("00000000-0000-4000-8000-000000000001");
	private static final UUID FAR = UUID.fromString("00000000-0000-4000-8000-000000000002");
	private static final Vec3 EYE = new Vec3(0, 1.62, 0);
	/** Looking along +x. */
	private static final Vec3 EAST = new Vec3(1, 0, 0);

	/** A standing player's hitbox with its feet at {@code (x, 0, z)}. */
	private static AABB player(double x, double z) {
		return new AABB(x - 0.3, 0, z - 0.3, x + 0.3, 1.8, z + 0.3);
	}

	@Test
	void picksThePlayerUnderTheCrosshair() {
		assertEquals(NEAR, LookTarget.pick(EYE, EAST, Map.of(NEAR, player(5, 0))));
	}

	@Test
	void theNearestWins() {
		assertEquals(NEAR, LookTarget.pick(EYE, EAST, Map.of(FAR, player(20, 0), NEAR, player(5, 0))));
	}

	@Test
	void aNarrowConeForgivesAimingBesideAFarPlayer() {
		// 1 block beside a player 20 blocks away is about 2° off its edge: inside the cone.
		assertEquals(FAR, LookTarget.pick(EYE, EAST, Map.of(FAR, player(20, 1))));
		// 3 blocks beside a player 10 blocks away is far outside it.
		assertNull(LookTarget.pick(EYE, EAST, Map.of(NEAR, player(10, 3))));
	}

	@Test
	void nobodyBehindOrOutOfReach() {
		assertNull(LookTarget.pick(EYE, EAST, Map.of(NEAR, player(-5, 0))));
		assertNull(LookTarget.pick(EYE, EAST, Map.of(FAR, player(LookTarget.MAX_DISTANCE + 2, 0))));
	}
}
