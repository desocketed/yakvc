package io.github.desocketed.yakvc;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import io.github.desocketed.yakvc.config.SoundTuning;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.world.phys.Vec3;
import org.junit.jupiter.api.Test;

class RoomTest {
	private static final SoundTuning TUNING = SoundTuning.DEFAULT;
	private static final double STONE = 0.02;
	private static final double EYE = 1.62;

	/** The test world's small stone room, 7 × 4 × 7 inside. */
	@Test
	void aSmallStoneRoomRingsBrieflyAndCloseVoicesStayDry() {
		Room room = box(7, 4, 7);
		assertBetween(0.7, 1.0, room.decay());
		assertTrue(room.criticalDistance() > 1, "critical distance " + room.criticalDistance());
		assertEquals(1, room.reverb(), 1e-6);
		assertEquals(0, room.echo());
	}

	/** The test world's stone hall, 19 × 11 × 19 inside. */
	@Test
	void aHallRingsLongerWithAnEcho() {
		Room room = box(19, 11, 19);
		assertBetween(2, 3, room.decay());
		assertTrue(room.echo() > 0.1, "echo " + room.echo());
	}

	/** The test world's cavern, 48 × 24 × 48 inside. */
	@Test
	void aCavernRingsForSecondsWithAnEcho() {
		Room room = box(48, 24, 48);
		assertBetween(4, 6, room.decay());
		assertTrue(room.echo() > 0.1, "echo " + room.echo());
		assertEquals(2 * 24 / 343.0, room.echoDelay(), 0.05);
	}

	@Test
	void softWallsRingShorter() {
		assertTrue(box(19, 11, 19, 0.5).decay() < box(19, 11, 19, STONE).decay() / 2);
	}

	@Test
	void openGroundStaysDry() {
		List<Room.Ray> rays = new ArrayList<>();
		for (Vec3 d : Room.directions()) {
			double ground = d.y < 0 ? EYE / -d.y : Double.MAX_VALUE;
			rays.add(ground <= TUNING.roomReach() ? new Room.Ray(d.y <= -0.3, true, ground, -d.y, STONE)
					: Room.Ray.miss(d.y <= -0.3));
		}
		Room room = Room.of(rays, TUNING);
		// Only the ground near the horizon counts, a little.
		assertTrue(room.reverb() < 0.15, "reverb " + room.reverb());
		assertEquals(0, room.echo());
	}

	@Test
	void decayIsCappedByTheTuning() {
		Room room = Room.of(rays(200, 100, 200, 0), new SoundTuning(0, 6, 128, 0, 0));
		assertEquals(6, room.decay(), 1e-6);
	}

	private static Room box(double width, double height, double depth) {
		return box(width, height, depth, STONE);
	}

	private static Room box(double width, double height, double depth, double absorption) {
		return Room.of(rays(width, height, depth, absorption), TUNING);
	}

	/** Rays from eye height in the middle of a closed box whose walls all absorb {@code absorption}. */
	private static List<Room.Ray> rays(double width, double height, double depth, double absorption) {
		double[] eye = {width / 2, EYE, depth / 2};
		double[] size = {width, height, depth};
		List<Room.Ray> rays = new ArrayList<>();
		for (Vec3 d : Room.directions()) {
			double[] v = {d.x, d.y, d.z};
			double nearest = Double.MAX_VALUE;
			double incidence = 1;
			for (int axis = 0; axis < 3; axis++) {
				if (Math.abs(v[axis]) < 1e-9) continue;
				double t = ((v[axis] > 0 ? size[axis] : 0) - eye[axis]) / v[axis];
				if (t < nearest) {
					nearest = t;
					incidence = Math.abs(v[axis]);
				}
			}
			rays.add(new Room.Ray(d.y <= -0.3, true, nearest, incidence, absorption));
		}
		return rays;
	}

	private static void assertBetween(double low, double high, double value) {
		assertTrue(value >= low && value <= high, value + " not in [" + low + ", " + high + "]");
	}
}
