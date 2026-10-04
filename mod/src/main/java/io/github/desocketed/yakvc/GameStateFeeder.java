package io.github.desocketed.yakvc;

import io.github.desocketed.yakvc.config.ClientConfig;
import io.github.desocketed.yakvc.input.VoiceKeys;
import io.github.desocketed.yakvc.natives.EngineEvent;
import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.natives.YakVcException;
import java.lang.foreign.MemorySegment;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;
import java.util.UUID;
import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.client.player.AbstractClientPlayer;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.sounds.SoundSource;
import net.minecraft.world.level.GameType;
import net.minecraft.world.phys.Vec3;
import org.jspecify.annotations.Nullable;

/**
 * Feeds the engine from the game on every client tick and drains its events. Owns the engine handle: everything here
 * runs on the client thread.
 */
public final class GameStateFeeder {
	/** Fits the largest record: an error message of 65,535 bytes plus its header. */
	private static final int EVENT_BUFFER_BYTES = 65_540;

	private final NativeBridge bridge;
	private final MemorySegment engine;
	private final ClientConfig config;
	private final VoiceKeys keys;
	private final byte[] eventBuffer = new byte[EVENT_BUFFER_BYTES];

	/** Volatile because Fabric may fire DISCONNECT off the client thread. */
	private volatile @Nullable VoiceSession session;
	/** The engine was destroyed, or crashed and can only be destroyed. */
	private boolean closed;

	// What the engine was last told, so unchanged values aren't sent again.
	private Set<UUID> tabList = Set.of();
	private List<UUID> tracked = List.of();
	private boolean worldEmpty = true;
	/** The engine starts with no input flags set. */
	private int inputFlags = 0;
	private float gameVolume = -1;
	private @Nullable String gameDevice;

	// Shown by the HUD.
	private EngineEvent.@Nullable RendezvousState rendezvous;
	private final Set<UUID> talking = new LinkedHashSet<>();
	private int errorEvents;

	public GameStateFeeder(NativeBridge bridge, MemorySegment engine, ClientConfig config, VoiceKeys keys) {
		this.bridge = bridge;
		this.engine = engine;
		this.config = config;
		this.keys = keys;
	}

	/** The local Minecraft identity. Starts rendezvous authentication. */
	public void setIdentity(UUID uuid, String name) {
		if (closed) return;
		try {
			bridge.setIdentity(engine, uuid, name);
		} catch (YakVcException e) {
			fail(e);
		}
	}

	public void join(ClientPacketListener connection, Minecraft minecraft) {
		// On an offline-mode dev server the game profile differs from the launcher's account; the tab list uses it.
		setIdentity(connection.getLocalGameProfile().id(), connection.getLocalGameProfile().name());
		session = new VoiceSession(connection, minecraft, config);
	}

	public void leave() {
		session = null;
	}

	public void tick(Minecraft minecraft) {
		if (closed) return;
		try {
			keys.tick();
			VoiceSession session = this.session;
			if (session != null) session.tick();
			else talking.clear();
			LocalPlayer player = minecraft.player;
			boolean active = session != null && session.active() && player != null && minecraft.level != null;

			feedTabList(active ? new HashSet<>(session.connection.getOnlinePlayerIds()) : Set.of());
			if (active) {
				feedWorld(minecraft, player, session.connection);
			} else if (!worldEmpty) {
				bridge.pushWorld(engine, new double[5], new byte[0], new double[0]);
				tracked = List.of();
				worldEmpty = true;
			}
			feedInput(active ? inputFlags(player) : 0);
			feedVolume(minecraft.options.getFinalSoundSourceVolume(SoundSource.VOICE));
			feedDevice(minecraft.options.soundDevice().get());
			drainEvents();
		} catch (YakVcException e) {
			fail(e);
		}
	}

	/** Shuts the engine down. Must be the last call. */
	public void close() {
		if (closed) return;
		closed = true;
		bridge.destroy(engine);
	}

	private void feedTabList(Set<UUID> players) {
		if (players.equals(tabList)) return;
		bridge.setTabList(engine, players);
		tabList = players;
	}

	/**
	 * Pushes the listener's pose and every tracked player except spectators. Vanilla sends spectator players to other
	 * clients and only hides them client-side, so the tab-list game mode is what keeps spectators out, both ways.
	 */
	private void feedWorld(Minecraft minecraft, LocalPlayer player, ClientPacketListener connection) {
		Camera camera = minecraft.gameRenderer.mainCamera();
		Vec3 eye = camera.isInitialized() ? camera.position() : player.getEyePosition();
		float yaw = camera.isInitialized() ? camera.yRot() : player.getYRot();
		float pitch = camera.isInitialized() ? camera.xRot() : player.getXRot();
		double[] listener = {eye.x, eye.y, eye.z, yaw, pitch};

		List<UUID> uuids = new ArrayList<>();
		List<Vec3> positions = new ArrayList<>();
		for (AbstractClientPlayer other : minecraft.level.players()) {
			PlayerInfo info = connection.getPlayerInfo(other.getUUID());
			if (other == player || info != null && info.getGameMode() == GameType.SPECTATOR) continue;
			uuids.add(other.getUUID());
			positions.add(other.getEyePosition());
		}
		double[] xyz = new double[3 * positions.size()];
		for (int i = 0; i < positions.size(); i++) {
			xyz[3 * i] = positions.get(i).x;
			xyz[3 * i + 1] = positions.get(i).y;
			xyz[3 * i + 2] = positions.get(i).z;
		}
		bridge.pushWorld(engine, listener, NativeBridge.uuidBytes(uuids), xyz);
		tracked = uuids;
		worldEmpty = false;
	}

	private int inputFlags(LocalPlayer player) {
		int flags = 0;
		if (keys.pushToTalkDown()) flags |= NativeBridge.INPUT_PUSH_TO_TALK;
		if (keys.muted()) flags |= NativeBridge.INPUT_MUTED;
		if (keys.deafened()) flags |= NativeBridge.INPUT_DEAFENED;
		if (player.isSpectator()) flags |= NativeBridge.INPUT_SPECTATOR;
		return flags;
	}

	private void feedInput(int flags) {
		if (flags == inputFlags) return;
		bridge.setInput(engine, flags);
		inputFlags = flags;
	}

	private void feedVolume(float volume) {
		volume = Math.clamp(volume, 0f, 1f);
		if (volume == gameVolume) return;
		bridge.setGameVolume(engine, volume);
		gameVolume = volume;
	}

	private void feedDevice(String device) {
		if (device.equals(gameDevice)) return;
		bridge.setGameDevice(engine, device);
		gameDevice = device;
	}

	private void drainEvents() {
		int written;
		while ((written = bridge.pollEvents(engine, eventBuffer)) > 0) {
			for (EngineEvent event : EngineEvent.decode(eventBuffer, written)) {
				handle(event);
			}
		}
	}

	private void handle(EngineEvent event) {
		switch (event) {
			case EngineEvent.JoinRequest(int id, String serverId) -> {
				// Real Mojang authentication (SessionJoiner) comes with M5; a dev rendezvous never asks.
				YakVcClient.LOGGER.warn("Refusing rendezvous join request {}: Mojang authentication is not built yet", id);
				bridge.completeJoin(engine, id, false);
			}
			case EngineEvent.Rendezvous(EngineEvent.RendezvousState state, long value) -> {
				rendezvous = state;
				YakVcClient.LOGGER.info("Rendezvous {} ({})", state, value);
			}
			case EngineEvent.Peer(UUID uuid, EngineEvent.PeerState state) -> {
				if (state == EngineEvent.PeerState.GONE) talking.remove(uuid);
				YakVcClient.LOGGER.info("Peer {} {} {}", uuid, name(uuid), state);
			}
			case EngineEvent.Talking(UUID uuid, boolean isTalking) -> {
				if (isTalking) talking.add(uuid);
				else talking.remove(uuid);
				YakVcClient.LOGGER.info("Talking {} {} {}", uuid, name(uuid), isTalking);
			}
			case EngineEvent.MicLevel(float db) -> {}
			case EngineEvent.Error(String message) -> {
				errorEvents++;
				YakVcClient.LOGGER.warn("Engine: {}", message);
			}
		}
	}

	private void fail(YakVcException e) {
		YakVcClient.LOGGER.error("Native call failed", e);
		if (e.poisoned()) {
			YakVcClient.LOGGER.error("Voice disabled until restart: the native engine crashed");
			close();
		}
	}

	/** The player's name if they are in the tab list, else "?". */
	public String name(UUID uuid) {
		VoiceSession session = this.session;
		PlayerInfo info = session == null ? null : session.connection.getPlayerInfo(uuid);
		return info == null ? "?" : info.getProfile().name();
	}

	public @Nullable VoiceSession session() {
		return session;
	}

	public boolean closed() {
		return closed;
	}

	public EngineEvent.@Nullable RendezvousState rendezvous() {
		return rendezvous;
	}

	/** Who is talking right now, the local player included. */
	public Set<UUID> talking() {
		return talking;
	}

	/** Players in the last world snapshot. For tests. */
	public List<UUID> tracked() {
		return tracked;
	}

	/** The last {@code NativeBridge.INPUT_*} flags sent. For tests. */
	public int inputFlags() {
		return inputFlags;
	}

	/** Engine {@code Error} events so far. For tests. */
	public int errorEvents() {
		return errorEvents;
	}
}
