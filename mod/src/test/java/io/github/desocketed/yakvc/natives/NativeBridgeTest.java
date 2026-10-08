package io.github.desocketed.yakvc.natives;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.lang.foreign.MemorySegment;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Set;
import java.util.UUID;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class NativeBridgeTest {
	/** No rendezvous, and test audio instead of devices. */
	private static final String DEV_CONFIG = """
			dev_mode = true
			[dev]
			tone_hz = 440.0
			null_output = true
			""";

	@Test
	void loadsHostLibraryAndMatchesAbi(@TempDir Path configDir) throws Exception {
		NativeBridge bridge = new NativeBridge(NativeLoader.load(configDir));
		assertEquals(NativeBridge.ABI_VERSION, bridge.abiVersion());
	}

	@Test
	void reusesExtractedLibrary(@TempDir Path configDir) throws Exception {
		NativeLoader.load(configDir);
		NativeBridge bridge = new NativeBridge(NativeLoader.load(configDir));
		assertEquals(NativeBridge.ABI_VERSION, bridge.abiVersion());
	}

	@Test
	void engineRunsAndReportsOwnTalking(@TempDir Path configDir) throws Exception {
		NativeBridge bridge = new NativeBridge(NativeLoader.load(configDir));
		MemorySegment engine = bridge.create(configDir.toString(), DEV_CONFIG);
		try {
			UUID me = UUID.fromString("00112233-4455-6677-8899-aabbccddeeff");
			UUID other = UUID.randomUUID();
			bridge.setIdentity(engine, me, "alice");
			bridge.setTabList(engine, Set.of(me, other));
			bridge.pushWorld(engine, new double[] {0, 64, 0, 90, 0}, NativeBridge.uuidBytes(List.of(other)),
					new double[] {3, 64, 0});
			bridge.setGameVolume(engine, 0.5f);
			bridge.setGameDevice(engine, "");
			bridge.setPeerVolume(engine, other, 1.5f, false);
			bridge.setInput(engine, NativeBridge.INPUT_PUSH_TO_TALK);

			List<EngineEvent> events = pollUntil(bridge, engine, new EngineEvent.Talking(me, true));
			assertTrue(events.contains(new EngineEvent.Rendezvous(EngineEvent.RendezvousState.DISCONNECTED, 0, false)),
					events.toString());
			assertTrue(bridge.listDevices(engine).startsWith("{"));
			VoiceStats stats = VoiceStats.parse(bridge.stats(engine));
			assertEquals(64, stats.endpointId().length());
			assertTrue(stats.peers().isEmpty(), stats.toString());
			bridge.updateConfig(engine, DEV_CONFIG);
		} finally {
			bridge.destroy(engine);
		}
	}

	@Test
	void joinsAndLeavesAGroup(@TempDir Path configDir) throws Exception {
		NativeBridge bridge = new NativeBridge(NativeLoader.load(configDir));
		MemorySegment engine = bridge.create(configDir.toString(), DEV_CONFIG);
		try {
			UUID me = UUID.fromString("00112233-4455-6677-8899-aabbccddeeff");
			bridge.setIdentity(engine, me, "alice");
			bridge.joinGroup(engine, "Miners", "pw");
			bridge.setGroupNearby(engine, false);
			String id = bridge.groupId("Miners", "pw");
			assertEquals(64, id.length());
			String groups = bridge.listGroups(engine);
			assertTrue(groups.contains("\"id\":\"" + id + "\""), groups);
			assertTrue(groups.contains(me.toString()), groups);
			assertTrue(!bridge.groupId("Miners", "other").equals(id));

			YakVcException blank = assertThrows(YakVcException.class, () -> bridge.joinGroup(engine, " ", ""));
			assertTrue(blank.getMessage().contains("1 to 32"), blank.getMessage());
			bridge.leaveGroup(engine);
			assertEquals("[]", bridge.listGroups(engine));
		} finally {
			bridge.destroy(engine);
		}
	}

	@Test
	void errorsCarryTheNativeMessage(@TempDir Path configDir) throws Exception {
		NativeBridge bridge = new NativeBridge(NativeLoader.load(configDir));
		YakVcException bad = assertThrows(YakVcException.class, () -> bridge.create(configDir.toString(), "voice_range = 0"));
		assertTrue(bad.getMessage().contains("voice_range"), bad.getMessage());

		MemorySegment engine = bridge.create(configDir.toString(), DEV_CONFIG);
		try {
			YakVcException e = assertThrows(YakVcException.class, () -> bridge.setGameVolume(engine, 2f));
			assertEquals(-1, e.code());
			assertTrue(e.getMessage().contains("outside 0..1"), e.getMessage());
		} finally {
			bridge.destroy(engine);
		}
	}

	private static List<EngineEvent> pollUntil(NativeBridge bridge, MemorySegment engine, EngineEvent wanted)
			throws InterruptedException {
		List<EngineEvent> seen = new ArrayList<>();
		byte[] buf = new byte[65_540];
		for (int i = 0; i < 100 && !seen.contains(wanted); i++) {
			Thread.sleep(50);
			int written;
			while ((written = bridge.pollEvents(engine, buf)) > 0) {
				seen.addAll(EngineEvent.decode(buf, written));
			}
		}
		assertTrue(seen.contains(wanted), "never saw " + wanted + " in " + seen);
		return seen;
	}
}
