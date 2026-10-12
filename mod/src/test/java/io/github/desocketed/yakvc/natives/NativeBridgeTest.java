package io.github.desocketed.yakvc.natives;

import static java.lang.foreign.ValueLayout.ADDRESS;
import static java.lang.foreign.ValueLayout.JAVA_BOOLEAN;
import static java.lang.foreign.ValueLayout.JAVA_INT;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import io.github.desocketed.yakvc.GameStateFeeder;
import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemorySegment;
import java.lang.invoke.MethodHandles;
import java.lang.invoke.MethodType;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicInteger;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.CleanupMode;
import org.junit.jupiter.api.io.TempDir;

// The loaded library is never unloaded, and Windows cannot delete a file that
// is still mapped, so these temp dirs are left for the OS to clean up.
class NativeBridgeTest {
	/** No rendezvous, and test audio instead of devices. */
	private static final String DEV_CONFIG = """
			dev_mode = true
			[dev]
			tone_hz = 440.0
			null_output = true
			""";

	@Test
	void loadsHostLibraryAndMatchesAbi(@TempDir(cleanup = CleanupMode.NEVER) Path configDir) throws Exception {
		NativeBridge bridge = new NativeBridge(NativeLoader.load(configDir));
		assertEquals(NativeBridge.ABI_VERSION, bridge.abiVersion());
	}

	@Test
	void reusesExtractedLibrary(@TempDir(cleanup = CleanupMode.NEVER) Path configDir) throws Exception {
		NativeLoader.load(configDir);
		NativeBridge bridge = new NativeBridge(NativeLoader.load(configDir));
		assertEquals(NativeBridge.ABI_VERSION, bridge.abiVersion());
	}

	@Test
	void engineRunsAndReportsOwnTalking(@TempDir(cleanup = CleanupMode.NEVER) Path configDir) throws Exception {
		NativeBridge bridge = new NativeBridge(NativeLoader.load(configDir));
		MemorySegment engine = bridge.create(configDir.toString(), DEV_CONFIG);
		try {
			UUID me = UUID.fromString("00112233-4455-6677-8899-aabbccddeeff");
			UUID other = UUID.randomUUID();
			bridge.setIdentity(engine, me, "alice");
			bridge.setTabList(engine, Set.of(me, other));
			bridge.pushWorld(engine, new double[] {0, 64, 0, 90, 0, 0, 1, 0, 0}, NativeBridge.uuidBytes(List.of(other)),
					new double[] {3, 64, 0, 0, 3, 64, 0});
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
	void groupCallsOutsideAGroup(@TempDir(cleanup = CleanupMode.NEVER) Path configDir) throws Exception {
		NativeBridge bridge = new NativeBridge(NativeLoader.load(configDir));
		MemorySegment engine = bridge.create(configDir.toString(), DEV_CONFIG);
		try {
			UUID me = UUID.fromString("00112233-4455-6677-8899-aabbccddeeff");
			UUID other = UUID.randomUUID();
			bridge.setIdentity(engine, me, "alice");
			// Inviting doesn't put us in a group until they accept.
			bridge.invite(engine, other);
			assertEquals("[]", bridge.listGroups(engine));

			assertUnavailable(() -> bridge.acceptInvite(engine, other));
			assertUnavailable(() -> bridge.joinGroup(engine, "0".repeat(64)));
			assertUnavailable(() -> bridge.setGroupPublic(engine, true));
			assertUnavailable(() -> bridge.setGroupLabel(engine, "Miners"));
			YakVcException notHex = assertThrows(YakVcException.class, () -> bridge.joinGroup(engine, "z".repeat(64)));
			assertEquals(-1, notHex.code());
			assertEquals(-1, assertThrows(YakVcException.class, () -> bridge.joinGroup(engine, "00")).code());
			bridge.leaveGroup(engine);
		} finally {
			bridge.destroy(engine);
		}
	}

	private static void assertUnavailable(Runnable call) {
		YakVcException e = assertThrows(YakVcException.class, call::run);
		assertEquals(NativeBridge.ERR_UNAVAILABLE, e.code(), e.getMessage());
	}

	@Test
	void errorsCarryTheNativeMessage(@TempDir(cleanup = CleanupMode.NEVER) Path configDir) throws Exception {
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

	@Test
	void pushWorldRejectsMismatchedLengths(@TempDir(cleanup = CleanupMode.NEVER) Path configDir) throws Exception {
		NativeBridge bridge = new NativeBridge(NativeLoader.load(configDir));
		MemorySegment engine = bridge.create(configDir.toString(), DEV_CONFIG);
		try {
			// Only over-long arrays, so native code can't read past one even without the check.
			double[] listener = {0, 64, 0, 90, 0, 0, 1, 0, 0};
			byte[] one = NativeBridge.uuidBytes(List.of(UUID.randomUUID()));
			double[] players = {3, 64, 0, 0, 3, 64, 0};
			assertThrows(IllegalArgumentException.class,
					() -> bridge.pushWorld(engine, new double[] {0, 64, 0, 90, 0, 0, 1, 0, 0, 0}, one, players));
			assertThrows(IllegalArgumentException.class,
					() -> bridge.pushWorld(engine, listener, Arrays.copyOf(one, 17), players));
			assertThrows(IllegalArgumentException.class,
					() -> bridge.pushWorld(engine, listener, one, new double[] {3, 64, 0, 0, 3, 64, 0, 4}));
			bridge.pushWorld(engine, listener, one, players);
		} finally {
			bridge.destroy(engine);
		}
	}

	/** How often the engine's audio thread asked the test voice sink for a frame. */
	private static final AtomicInteger sinkAsked = new AtomicInteger();

	private static boolean wantsFrame(MemorySegment user) {
		sinkAsked.incrementAndGet();
		return false;
	}

	private static void ignore(MemorySegment user, MemorySegment arg) {}

	@Test
	void voiceSinkIsCheckedAskedAndCleared(@TempDir(cleanup = CleanupMode.NEVER) Path configDir) throws Exception {
		NativeBridge bridge = new NativeBridge(NativeLoader.load(configDir));
		MemorySegment engine = bridge.create(configDir.toString(), DEV_CONFIG);
		try {
			// A sink must stay valid for the rest of the process, so it lives in the global arena.
			Linker linker = Linker.nativeLinker();
			MethodHandles.Lookup lookup = MethodHandles.lookup();
			MemorySegment wantsFrame = linker.upcallStub(lookup.findStatic(NativeBridgeTest.class, "wantsFrame",
					MethodType.methodType(boolean.class, MemorySegment.class)), FunctionDescriptor.of(JAVA_BOOLEAN, ADDRESS),
					Arena.global());
			MemorySegment ignore = linker.upcallStub(lookup.findStatic(NativeBridgeTest.class, "ignore",
					MethodType.methodType(void.class, MemorySegment.class, MemorySegment.class)),
					FunctionDescriptor.ofVoid(ADDRESS, ADDRESS), Arena.global());
			// YakVcVoiceSink: the version, then the user pointer and the three callbacks, 8-byte aligned.
			MemorySegment sink = Arena.global().allocate(40, 8);
			sink.set(JAVA_INT, 0, 1);
			sink.set(ADDRESS, 8, MemorySegment.NULL);
			sink.set(ADDRESS, 16, wantsFrame);
			sink.set(ADDRESS, 24, ignore);
			sink.set(ADDRESS, 32, ignore);

			bridge.setVoiceSink(engine, sink);
			awaitExternalOutput(bridge, engine, true);
			assertTrue(sinkAsked.get() > 0, "the sink was never asked for a frame");
			bridge.setVoiceSink(engine, MemorySegment.NULL);
			awaitExternalOutput(bridge, engine, false);

			// The version yakvc publishes for renderers is the newest the engine takes; a newer one isn't.
			sink.set(JAVA_INT, 0, GameStateFeeder.VOICE_SINK_VERSION);
			bridge.setVoiceSink(engine, sink);
			bridge.setVoiceSink(engine, MemorySegment.NULL);
			sink.set(JAVA_INT, 0, GameStateFeeder.VOICE_SINK_VERSION + 1);
			YakVcException wrongVersion = assertThrows(YakVcException.class, () -> bridge.setVoiceSink(engine, sink));
			assertEquals(-1, wrongVersion.code());
			assertTrue(wrongVersion.getMessage().contains("version"), wrongVersion.getMessage());
		} finally {
			bridge.destroy(engine);
		}
	}

	private static void awaitExternalOutput(NativeBridge bridge, MemorySegment engine, boolean external)
			throws InterruptedException {
		for (int i = 0; i < 100; i++) {
			if (VoiceStats.parse(bridge.stats(engine)).audio().externalOutput() == external) return;
			Thread.sleep(50);
		}
		throw new AssertionError("external output never became " + external);
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
