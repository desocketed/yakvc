package io.github.desocketed.yakvc.natives;

import static java.lang.foreign.ValueLayout.ADDRESS;
import static java.lang.foreign.ValueLayout.JAVA_BOOLEAN;
import static java.lang.foreign.ValueLayout.JAVA_BYTE;
import static java.lang.foreign.ValueLayout.JAVA_FLOAT;
import static java.lang.foreign.ValueLayout.JAVA_INT;
import static java.lang.foreign.ValueLayout.JAVA_LONG;

import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.util.Collection;
import java.util.List;
import java.util.UUID;

/**
 * Downcall handles for the C ABI in {@code crates/yakvc-ffi/include/yakvc.h}, plus thin typed wrappers that turn
 * error codes into {@link YakVcException}.
 *
 * <p>{@code engine} arguments are the handle from {@link #create}. Strings and small buffers live in a confined arena
 * for one call. The two per-tick calls, {@link #pushWorld} and {@link #pollEvents}, are linked as critical so they
 * can take Java arrays without copying.
 */
public final class NativeBridge {
	/** Must equal {@code YAKVC_ABI_VERSION} in {@code yakvc.h}. */
	public static final int ABI_VERSION = 5;

	public static final int INPUT_PUSH_TO_TALK = 1;
	public static final int INPUT_MUTED = 1 << 1;
	public static final int INPUT_DEAFENED = 1 << 2;
	public static final int INPUT_SPECTATOR = 1 << 3;

	public static final int ERR_PANIC = -6;
	public static final int ERR_POISONED = -7;
	/** A group action that cannot be done now, such as accepting an invite that expired. */
	public static final int ERR_UNAVAILABLE = -8;

	// size_t is 64 bits on every platform Minecraft runs on.
	private static final ValueLayout.OfLong SIZE_T = JAVA_LONG;

	private final MethodHandle abiVersion;
	private final MethodHandle create;
	private final MethodHandle destroy;
	private final MethodHandle setIdentity;
	private final MethodHandle setTabList;
	private final MethodHandle pushWorld;
	private final MethodHandle setInput;
	private final MethodHandle setGameVolume;
	private final MethodHandle setGameDevice;
	private final MethodHandle setPeerVolume;
	private final MethodHandle completeJoin;
	private final MethodHandle pollEvents;
	private final MethodHandle updateConfig;
	private final MethodHandle listDevices;
	private final MethodHandle stats;
	private final MethodHandle invite;
	private final MethodHandle acceptInvite;
	private final MethodHandle joinGroup;
	private final MethodHandle leaveGroup;
	private final MethodHandle setGroupPublic;
	private final MethodHandle setGroupLabel;
	private final MethodHandle voteKick;
	private final MethodHandle listGroups;
	private final MethodHandle lastError;

	public NativeBridge(SymbolLookup library) {
		Linker linker = Linker.nativeLinker();
		Linker.Option critical = Linker.Option.critical(true);
		abiVersion = linker.downcallHandle(find(library, "yakvc_abi_version"), FunctionDescriptor.of(JAVA_INT));
		create = linker.downcallHandle(find(library, "yakvc_create"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, SIZE_T, ADDRESS, SIZE_T, JAVA_INT, ADDRESS));
		destroy = linker.downcallHandle(find(library, "yakvc_destroy"), FunctionDescriptor.ofVoid(ADDRESS));
		setIdentity = linker.downcallHandle(find(library, "yakvc_set_identity"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, ADDRESS, SIZE_T));
		setTabList = linker.downcallHandle(find(library, "yakvc_set_tab_list"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, SIZE_T));
		pushWorld = linker.downcallHandle(find(library, "yakvc_push_world"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, ADDRESS, ADDRESS, SIZE_T), critical);
		setInput = linker.downcallHandle(find(library, "yakvc_set_input"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, JAVA_INT));
		setGameVolume = linker.downcallHandle(find(library, "yakvc_set_game_volume"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, JAVA_FLOAT));
		setGameDevice = linker.downcallHandle(find(library, "yakvc_set_game_device"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, SIZE_T));
		setPeerVolume = linker.downcallHandle(find(library, "yakvc_set_peer_volume"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, JAVA_FLOAT, JAVA_BOOLEAN));
		completeJoin = linker.downcallHandle(find(library, "yakvc_complete_join"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, JAVA_INT, JAVA_BOOLEAN));
		pollEvents = linker.downcallHandle(find(library, "yakvc_poll_events"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, SIZE_T, ADDRESS), critical);
		updateConfig = linker.downcallHandle(find(library, "yakvc_update_config"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, SIZE_T));
		listDevices = linker.downcallHandle(find(library, "yakvc_list_devices"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, SIZE_T, ADDRESS));
		stats = linker.downcallHandle(find(library, "yakvc_stats"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, SIZE_T, ADDRESS));
		invite = linker.downcallHandle(find(library, "yakvc_invite"), FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS));
		acceptInvite = linker.downcallHandle(find(library, "yakvc_accept_invite"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS));
		joinGroup = linker.downcallHandle(find(library, "yakvc_join_group"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS));
		leaveGroup = linker.downcallHandle(find(library, "yakvc_leave_group"), FunctionDescriptor.of(JAVA_INT, ADDRESS));
		setGroupPublic = linker.downcallHandle(find(library, "yakvc_set_group_public"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, JAVA_BOOLEAN));
		setGroupLabel = linker.downcallHandle(find(library, "yakvc_set_group_label"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, SIZE_T));
		voteKick = linker.downcallHandle(find(library, "yakvc_vote_kick"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, JAVA_BOOLEAN));
		listGroups = linker.downcallHandle(find(library, "yakvc_list_groups"),
				FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, SIZE_T, ADDRESS));
		lastError = linker.downcallHandle(find(library, "yakvc_last_error"),
				FunctionDescriptor.of(SIZE_T, ADDRESS, SIZE_T));
	}

	/** Returns the library's {@code YAKVC_ABI_VERSION}. */
	public int abiVersion() {
		return (int) invoke(abiVersion);
	}

	/** Starts an engine. {@code configDir} holds the client key and ticket cache. */
	public MemorySegment create(String configDir, String configToml) {
		try (Arena arena = Arena.ofConfined()) {
			MemorySegment dir = utf8(arena, configDir);
			MemorySegment toml = utf8(arena, configToml);
			MemorySegment out = arena.allocate(ADDRESS);
			check((int) invoke(create, dir, dir.byteSize(), toml, toml.byteSize(), ABI_VERSION, out));
			return out.get(ADDRESS, 0);
		}
	}

	/** Shuts the engine down (at most 500 ms). Must be the last call on {@code engine}. */
	public void destroy(MemorySegment engine) {
		invoke(destroy, engine);
	}

	public void setIdentity(MemorySegment engine, UUID uuid, String name) {
		try (Arena arena = Arena.ofConfined()) {
			MemorySegment nameBytes = utf8(arena, name);
			MemorySegment uuidBytes = arena.allocateFrom(JAVA_BYTE, uuidBytes(List.of(uuid)));
			check((int) invoke(setIdentity, engine, uuidBytes, nameBytes, nameBytes.byteSize()));
		}
	}

	/** Replaces the tab list. Call only when it changes. */
	public void setTabList(MemorySegment engine, Collection<UUID> uuids) {
		try (Arena arena = Arena.ofConfined()) {
			MemorySegment bytes = arena.allocateFrom(JAVA_BYTE, uuidBytes(uuids));
			check((int) invoke(setTabList, engine, bytes, bytes.byteSize() / 16));
		}
	}

	/**
	 * Pushes one tick's world.
	 *
	 * @param listener x, y, z, yaw, pitch
	 * @param uuids    16 bytes per tracked player, from {@link #uuidBytes}
	 * @param xyz      x, y, z per tracked player
	 * @throws IllegalArgumentException if the lengths don't match, because native code trusts them
	 */
	public void pushWorld(MemorySegment engine, double[] listener, byte[] uuids, double[] xyz) {
		long count = uuids.length / 16;
		if (listener.length != 5 || uuids.length % 16 != 0 || xyz.length != 3 * count) {
			throw new IllegalArgumentException("pushWorld: listener " + listener.length + ", uuids " + uuids.length
					+ " bytes, xyz " + xyz.length);
		}
		check((int) invoke(pushWorld, engine, MemorySegment.ofArray(listener), MemorySegment.ofArray(uuids),
				MemorySegment.ofArray(xyz), count));
	}

	/** {@code flags} is a combination of {@code INPUT_*}. */
	public void setInput(MemorySegment engine, int flags) {
		check((int) invoke(setInput, engine, flags));
	}

	/** The game's Voice/Speech volume, 0 to 1. */
	public void setGameVolume(MemorySegment engine, float volume) {
		check((int) invoke(setGameVolume, engine, volume));
	}

	/** The game's selected sound device; an empty name means the system default. */
	public void setGameDevice(MemorySegment engine, String name) {
		try (Arena arena = Arena.ofConfined()) {
			MemorySegment bytes = utf8(arena, name);
			check((int) invoke(setGameDevice, engine, bytes, bytes.byteSize()));
		}
	}

	public void setPeerVolume(MemorySegment engine, UUID uuid, float volume, boolean muted) {
		try (Arena arena = Arena.ofConfined()) {
			MemorySegment bytes = arena.allocateFrom(JAVA_BYTE, uuidBytes(List.of(uuid)));
			check((int) invoke(setPeerVolume, engine, bytes, volume, muted));
		}
	}

	/** Reports the result of {@code joinServer} for a {@link EngineEvent.JoinRequest}. */
	public void completeJoin(MemorySegment engine, int requestId, boolean ok) {
		check((int) invoke(completeJoin, engine, requestId, ok));
	}

	/** Fills {@code buf} with whole event records and returns the bytes written; 0 when none are queued. */
	public int pollEvents(MemorySegment engine, byte[] buf) {
		long[] written = new long[1];
		check((int) invoke(pollEvents, engine, MemorySegment.ofArray(buf), (long) buf.length,
				MemorySegment.ofArray(written)));
		return (int) written[0];
	}

	/** Applies a new {@code client.toml}. */
	public void updateConfig(MemorySegment engine, String toml) {
		try (Arena arena = Arena.ofConfined()) {
			MemorySegment bytes = utf8(arena, toml);
			check((int) invoke(updateConfig, engine, bytes, bytes.byteSize()));
		}
	}

	/** Returns the audio devices as JSON. */
	public String listDevices(MemorySegment engine) {
		return readJson(listDevices, engine);
	}

	/** Returns the diagnostics snapshot as JSON; {@code yakvc_stats} in {@code yakvc.h} lists the fields. */
	public String stats(MemorySegment engine) {
		return readJson(stats, engine);
	}

	/**
	 * Invites {@code target} to our group. Outside a group the invite is to a new one, which we join only once they
	 * accept.
	 */
	public void invite(MemorySegment engine, UUID target) {
		try (Arena arena = Arena.ofConfined()) {
			check((int) invoke(invite, engine, arena.allocateFrom(JAVA_BYTE, uuidBytes(List.of(target)))));
		}
	}

	/**
	 * Joins the group of the last invite from {@code from}, leaving any other. Fails with {@link #ERR_UNAVAILABLE} if
	 * there was none in the last two minutes.
	 */
	public void acceptInvite(MemorySegment engine, UUID from) {
		try (Arena arena = Arena.ofConfined()) {
			check((int) invoke(acceptInvite, engine, arena.allocateFrom(JAVA_BYTE, uuidBytes(List.of(from)))));
		}
	}

	/** Joins a public group from {@link #listGroups} by its id, 64 hex characters, leaving any other. */
	public void joinGroup(MemorySegment engine, String id) {
		byte[] bytes = id.getBytes(StandardCharsets.US_ASCII);
		// Native code reads exactly 64 bytes.
		if (bytes.length != 64) throw new IllegalArgumentException("a group id is 64 hex characters: " + id);
		try (Arena arena = Arena.ofConfined()) {
			check((int) invoke(joinGroup, engine, arena.allocateFrom(JAVA_BYTE, bytes)));
		}
	}

	public void leaveGroup(MemorySegment engine) {
		check((int) invoke(leaveGroup, engine));
	}

	/** Makes our group public (anyone may join) or private. */
	public void setGroupPublic(MemorySegment engine, boolean isPublic) {
		check((int) invoke(setGroupPublic, engine, isPublic));
	}

	/** Sets our group's label, at most 32 characters; empty for none. */
	public void setGroupLabel(MemorySegment engine, String label) {
		try (Arena arena = Arena.ofConfined()) {
			MemorySegment bytes = utf8(arena, label);
			check((int) invoke(setGroupLabel, engine, bytes, bytes.byteSize()));
		}
	}

	/** Votes on kicking {@code target}, a group mate, out of our group. */
	public void voteKick(MemorySegment engine, UUID target, boolean yes) {
		try (Arena arena = Arena.ofConfined()) {
			check((int) invoke(voteKick, engine, arena.allocateFrom(JAVA_BYTE, uuidBytes(List.of(target))), yes));
		}
	}

	/** Returns the known groups as JSON; {@code yakvc_list_groups} in {@code yakvc.h} lists the fields. */
	public String listGroups(MemorySegment engine) {
		return readJson(listGroups, engine);
	}

	/** Calls a function that writes JSON with the {@code (buf, cap, needed)} protocol, growing the buffer as asked. */
	private String readJson(MethodHandle handle, MemorySegment engine) {
		try (Arena arena = Arena.ofConfined()) {
			MemorySegment needed = arena.allocate(SIZE_T);
			long cap = 4096;
			while (true) {
				MemorySegment buf = arena.allocate(cap);
				check((int) invoke(handle, engine, buf, cap, needed));
				long length = needed.get(SIZE_T, 0);
				if (length <= cap) {
					return new String(buf.asSlice(0, length).toArray(JAVA_BYTE), StandardCharsets.UTF_8);
				}
				cap = length;
			}
		}
	}

	/** Packs UUIDs as the ABI wants them: 16 bytes each, most significant half first. */
	public static byte[] uuidBytes(Collection<UUID> uuids) {
		ByteBuffer buf = ByteBuffer.allocate(16 * uuids.size());
		for (UUID uuid : uuids) {
			buf.putLong(uuid.getMostSignificantBits()).putLong(uuid.getLeastSignificantBits());
		}
		return buf.array();
	}

	/** Throws with this thread's {@code yakvc_last_error} message unless {@code code} is 0. */
	private void check(int code) {
		if (code == 0) return;
		try (Arena arena = Arena.ofConfined()) {
			long cap = 1024;
			MemorySegment buf = arena.allocate(cap);
			long length = Math.min((long) invoke(lastError, buf, cap), cap);
			throw new YakVcException(code,
					new String(buf.asSlice(0, length).toArray(JAVA_BYTE), StandardCharsets.UTF_8));
		}
	}

	/** Calls {@code handle}. A downcall never throws, so anything thrown is a bug in this class. */
	private static Object invoke(MethodHandle handle, Object... args) {
		// invokeWithArguments boxes, which is fine at about a hundred calls a second.
		try {
			return handle.invokeWithArguments(args);
		} catch (Throwable t) {
			throw new IllegalStateException("bad downcall " + handle.type(), t);
		}
	}

	private static MemorySegment utf8(Arena arena, String s) {
		return arena.allocateFrom(JAVA_BYTE, s.getBytes(StandardCharsets.UTF_8));
	}

	private static MemorySegment find(SymbolLookup library, String name) {
		return library.find(name).orElseThrow(() -> new UnsatisfiedLinkError("missing native symbol " + name));
	}
}
