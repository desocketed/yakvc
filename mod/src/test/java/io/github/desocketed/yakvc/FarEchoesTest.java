package io.github.desocketed.yakvc;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.world.phys.Vec3;
import org.junit.jupiter.api.Test;

class FarEchoesTest {
	private static final int RAYS = 128;
	private static final double STONE = 0.02;

	/** The test world's canyon: stone cliffs 35.5 m to the west and 34.5 m to the east, 30 tall, 80 long. */
	@Test
	void eachCanyonCliffSendsItsOwnEcho() {
		List<FarEchoes.Hit> hits = new ArrayList<>();
		hits.addAll(cliff(-1, 35.5));
		hits.addAll(cliff(1, 34.5));
		List<FarEchoes.Echo> echoes = FarEchoes.pick(hits, RAYS);
		assertEquals(2, echoes.size(), echoes.toString());
		for (FarEchoes.Echo echo : echoes) {
			double distance = Math.abs(echo.direction().x) > 0.9 ? (echo.direction().x < 0 ? 35.5 : 34.5) : -1;
			assertEquals(2 * distance / 343, echo.delay(), 0.005, echo.toString());
			// 0.5 × 8.5 / d × √0.98: about 0.12.
			assertEquals(0.5 * 8.5 / distance * Math.sqrt(1 - STONE), echo.gain(), 0.005);
			assertTrue(echo.cutoff() > 4000 && echo.cutoff() < 5500, "cutoff " + echo.cutoff());
		}
		assertTrue(echoes.get(0).direction().x * echoes.get(1).direction().x < 0, "one from each side");
	}

	@Test
	void nearWallsAndGroundStretchingAwayAreLeftOut() {
		List<FarEchoes.Hit> hits = new ArrayList<>(cliff(1, 12));
		// Ground met at a glance, far away.
		hits.add(new FarEchoes.Hit(new Vec3(0, -0.05, 1).normalize(), 32, 0.05, 0.15));
		// A far wall facing away.
		hits.add(new FarEchoes.Hit(new Vec3(0, 0, -1), 60, -1, STONE));
		assertEquals(List.of(), FarEchoes.pick(hits, RAYS));
	}

	@Test
	void keepsTheStrongestFour() {
		List<FarEchoes.Hit> hits = new ArrayList<>();
		for (int i = 0; i < 8; i++) {
			double angle = i * Math.PI / 4;
			hits.addAll(cliff(new Vec3(Math.cos(angle), 0, Math.sin(angle)), 20 + 10 * i));
		}
		List<FarEchoes.Echo> echoes = FarEchoes.pick(hits, RAYS);
		assertEquals(FarEchoes.COUNT, echoes.size());
		// The nearest are the loudest.
		for (FarEchoes.Echo echo : echoes) assertTrue(echo.delay() < 2 * 55 / 343.0, echo.toString());
	}

	@Test
	void aLonePillarEchoesLessThanABroadWall() {
		double pillar = FarEchoes.pick(List.of(new FarEchoes.Hit(new Vec3(1, 0, 0), 40, 1, STONE)), RAYS).getFirst()
				.gain();
		double wall = FarEchoes.pick(cliff(1, 40), RAYS).getFirst().gain();
		assertTrue(pillar < wall / 2, "pillar " + pillar + ", wall " + wall);
	}

	private static List<FarEchoes.Hit> cliff(double side, double distance) {
		return cliff(new Vec3(side, 0, 0), distance);
	}

	/**
	 * What rays find on a flat cliff {@code distance} away whose face looks back along {@code normal}: rays up to 30°
	 * either side and 20° up and down of head on, as a big cliff fills them.
	 */
	private static List<FarEchoes.Hit> cliff(Vec3 toward, double distance) {
		List<FarEchoes.Hit> hits = new ArrayList<>();
		Vec3 side = new Vec3(-toward.z, 0, toward.x);
		for (int across = -2; across <= 2; across++) {
			for (int up = -1; up <= 1; up++) {
				Vec3 direction = toward.add(side.scale(Math.tan(Math.toRadians(15 * across))))
						.add(0, Math.tan(Math.toRadians(20 * up)), 0).normalize();
				double incidence = direction.dot(toward);
				hits.add(new FarEchoes.Hit(direction, distance / incidence, incidence, STONE));
			}
		}
		return hits;
	}
}
