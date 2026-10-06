package io.github.desocketed.yakvc;

import com.google.gson.JsonElement;
import com.google.gson.JsonParser;
import io.github.desocketed.yakvc.config.ClientConfig;
import io.github.desocketed.yakvc.config.PlayerVolumes;
import io.github.desocketed.yakvc.input.VoiceKeys;
import io.github.desocketed.yakvc.natives.EngineEvent;
import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.natives.YakVcException;
import io.github.desocketed.yakvc.ui.VoiceMenuScreen;
import io.github.desocketed.yakvc.ui.VoiceToasts;
import java.lang.foreign.MemorySegment;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Iterator;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.client.player.AbstractClientPlayer;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.network.chat.Component;
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
	/** How often the Social Interactions block list is checked again. */
	private static final int BLOCKED_REFRESH_TICKS = 20;

	private final NativeBridge bridge;
	private final MemorySegment engine;
	private final VoiceKeys keys;
	private final SessionJoiner joiner;
	private final PlayerVolumes volumes;
	private final byte[] eventBuffer = new byte[EVENT_BUFFER_BYTES];
	private ClientConfig config;

	/** Volatile because Fabric may fire DISCONNECT off the client thread. */
	private volatile @Nullable VoiceSession session;
	/** The engine was destroyed, or crashed and can only be destroyed. */
	private boolean closed;
	/**
	 * {@code joinServer} calls not yet answered. They are answered from {@link #tick} rather than the worker, so no
	 * native call can race {@link #close}.
	 */
	private final List<PendingJoin> pendingJoins = new ArrayList<>();

	private record PendingJoin(int id, CompletableFuture<Boolean> ok) {}

	// What the engine was last told, so unchanged values aren't sent again.
	private Set<UUID> tabList = Set.of();
	private List<UUID> tracked = List.of();
	private boolean worldEmpty = true;
	/** The engine starts with no input flags set. */
	private int inputFlags = 0;
	private float gameVolume = -1;
	private @Nullable String gameDevice;
	private final Map<UUID, PlayerVolumes.Setting> peerAudio = new HashMap<>();

	/** Players in the tab list that are blocked in Social Interactions, when {@code mute_blocked_players} is on. */
	private Set<UUID> blocked = Set.of();
	private int ticksUntilBlockedRefresh;

	// Shown by the HUD and the voice menu.
	private EngineEvent.@Nullable RendezvousState rendezvous;
	private final Map<UUID, EngineEvent.PeerState> peers = new HashMap<>();
	private final Set<UUID> talking = new LinkedHashSet<>();
	private int errorEvents;
	private boolean micHeard;

	public GameStateFeeder(NativeBridge bridge, MemorySegment engine, ClientConfig config, VoiceKeys keys,
			SessionJoiner joiner, PlayerVolumes volumes) {
		this.bridge = bridge;
		this.engine = engine;
		this.config = config;
		this.keys = keys;
		this.joiner = joiner;
		this.volumes = volumes;
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

	/**
	 * Every step runs on its own, so one that fails (a value the engine rejects, a bug) can't stop the rest: events
	 * are always drained and joins always answered, or the event queue grows and the rendezvous never signs in.
	 */
	public void tick(Minecraft minecraft) {
		step(() -> feedGame(minecraft));
		step(this::drainEvents);
		step(this::answerJoins);
	}

	private void feedGame(Minecraft minecraft) {
		keys.tick();
		if (keys.menuPressed() && minecraft.gui.screen() == null) {
			minecraft.gui.setScreen(new VoiceMenuScreen(null, this));
		}
		VoiceSession session = this.session;
		if (session != null) session.tick();
		else talking.clear();
		LocalPlayer player = minecraft.player;
		boolean active = session != null && session.active() && player != null && minecraft.level != null;

		Set<UUID> players = active ? new HashSet<>(session.connection.getOnlinePlayerIds()) : Set.of();
		refreshBlocked(minecraft, players);
		step(() -> feedTabList(players));
		step(() -> {
			if (active) feedWorld(minecraft, player, session.connection);
			else clearWorld();
		});
		feedPeerAudio(players);
		step(() -> feedInput(active ? inputFlags(player) : 0));
		step(() -> feedVolume(minecraft.options.getFinalSoundSourceVolume(SoundSource.VOICE)));
		step(() -> feedDevice(minecraft.options.soundDevice().get()));
	}

	/**
	 * Runs one step of the tick. A poisoned engine is shut down as before; any other failure is logged and the tick
	 * goes on. The feed methods record a value before sending it, so a value the engine rejected isn't sent again
	 * every tick, only once it changes.
	 */
	private void step(Runnable step) {
		if (closed) return;
		try {
			step.run();
		} catch (YakVcException e) {
			fail(e);
		} catch (RuntimeException e) {
			YakVcClient.LOGGER.error("Voice tick step failed", e);
		}
	}

	/** Shuts the engine down. Must be the last call. */
	public void close() {
		if (closed) return;
		closed = true;
		bridge.destroy(engine);
	}

	/**
	 * Applies a changed config to the running engine, which checks it. Returns the engine's complaint, or null if it
	 * took the config; only then may it be saved.
	 */
	public @Nullable String applyConfig(ClientConfig newConfig) {
		if (closed) return "the voice engine is stopped";
		try {
			bridge.updateConfig(engine, newConfig.toml());
		} catch (YakVcException e) {
			fail(e);
			return e.getMessage();
		}
		config = newConfig;
		ticksUntilBlockedRefresh = 0;
		return null;
	}

	/** The audio devices' names, inputs or outputs. Empty if the system can't list them. */
	public List<String> deviceNames(boolean inputs) {
		if (closed) return List.of();
		List<String> names = new ArrayList<>();
		try {
			JsonElement devices = JsonParser.parseString(bridge.listDevices(engine));
			for (JsonElement device : devices.getAsJsonObject().getAsJsonArray(inputs ? "inputs" : "outputs")) {
				names.add(device.getAsJsonObject().get("name").getAsString());
			}
		} catch (YakVcException e) {
			if (e.poisoned()) fail(e);
			else YakVcClient.LOGGER.warn("Could not list audio devices: {}", e.getMessage());
		}
		return names;
	}

	private void refreshBlocked(Minecraft minecraft, Set<UUID> players) {
		if (--ticksUntilBlockedRefresh > 0) return;
		ticksUntilBlockedRefresh = BLOCKED_REFRESH_TICKS;
		Set<UUID> now = new HashSet<>();
		if (config.muteBlockedPlayers()) {
			for (UUID uuid : players) {
				if (minecraft.getPlayerSocialManager().isBlocked(uuid)) now.add(uuid);
			}
		}
		if (!now.equals(blocked)) YakVcClient.LOGGER.info("Blocked players muted: {}", now.size());
		blocked = now;
	}

	private void feedTabList(Set<UUID> players) {
		if (players.equals(tabList)) return;
		tabList = players;
		bridge.setTabList(engine, players);
	}

	private void clearWorld() {
		if (worldEmpty) return;
		tracked = List.of();
		worldEmpty = true;
		bridge.pushWorld(engine, new double[5], new byte[0], new double[0]);
	}

	/**
	 * Pushes the listener's pose and every tracked player except spectators and blocked players. Vanilla sends
	 * spectator players to other clients and only hides them client-side, so the tab-list game mode is what keeps
	 * spectators out, both ways. Leaving blocked players out is what stops us sending to them; their mute stops them
	 * sending to us.
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
			if (blocked.contains(other.getUUID())) continue;
			uuids.add(other.getUUID());
			positions.add(other.getEyePosition());
		}
		double[] xyz = new double[3 * positions.size()];
		for (int i = 0; i < positions.size(); i++) {
			xyz[3 * i] = positions.get(i).x;
			xyz[3 * i + 1] = positions.get(i).y;
			xyz[3 * i + 2] = positions.get(i).z;
		}
		tracked = uuids;
		worldEmpty = false;
		bridge.pushWorld(engine, listener, NativeBridge.uuidBytes(uuids), xyz);
	}

	/** Each player's saved volume and mute, with blocked players muted. One player's failure doesn't skip the rest. */
	private void feedPeerAudio(Set<UUID> players) {
		for (UUID uuid : players) {
			PlayerVolumes.Setting saved = volumes.get(uuid);
			PlayerVolumes.Setting wanted = new PlayerVolumes.Setting(saved.volume(), saved.muted() || blocked(uuid));
			if (wanted.equals(peerAudio.getOrDefault(uuid, PlayerVolumes.DEFAULT))) continue;
			peerAudio.put(uuid, wanted);
			step(() -> bridge.setPeerVolume(engine, uuid, wanted.volume(), wanted.muted()));
		}
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
		inputFlags = flags;
		bridge.setInput(engine, flags);
	}

	private void feedVolume(float volume) {
		volume = Math.clamp(volume, 0f, 1f);
		if (volume == gameVolume) return;
		gameVolume = volume;
		bridge.setGameVolume(engine, volume);
	}

	private void feedDevice(String device) {
		if (device.equals(gameDevice)) return;
		gameDevice = device;
		bridge.setGameDevice(engine, device);
	}

	private void drainEvents() {
		int written;
		while (!closed && (written = bridge.pollEvents(engine, eventBuffer)) > 0) {
			for (EngineEvent event : EngineEvent.decode(eventBuffer, written)) {
				step(() -> handle(event));
			}
		}
	}

	private void handle(EngineEvent event) {
		switch (event) {
			case EngineEvent.JoinRequest(int id, String serverId) ->
					pendingJoins.add(new PendingJoin(id, joiner.joinAsync(serverId)));
			case EngineEvent.Rendezvous(EngineEvent.RendezvousState state, long value) -> {
				rendezvous = state;
				YakVcClient.LOGGER.info("Rendezvous {} ({})", state, value);
			}
			case EngineEvent.Peer(UUID uuid, EngineEvent.PeerState state) -> {
				if (state == EngineEvent.PeerState.GONE) {
					talking.remove(uuid);
					peers.remove(uuid);
				} else {
					peers.put(uuid, state);
				}
				YakVcClient.LOGGER.info("Peer {} {} {}", uuid, name(uuid), state);
			}
			case EngineEvent.Talking(UUID uuid, boolean isTalking) -> {
				if (isTalking) talking.add(uuid);
				else talking.remove(uuid);
				YakVcClient.LOGGER.info("Talking {} {} {}", uuid, name(uuid), isTalking);
			}
			case EngineEvent.MicLevel(float db) -> {
				// Levels only arrive while the microphone (or the dev test tone) delivers audio, so the first one
				// shows in the log that capture is working.
				if (!micHeard) {
					micHeard = true;
					YakVcClient.LOGGER.info("Microphone is delivering audio ({} dBFS)", db);
				}
			}
			case EngineEvent.Error(String message) -> {
				errorEvents++;
				YakVcClient.LOGGER.warn("Engine: {}", message);
			}
		}
	}

	private void answerJoins() {
		Iterator<PendingJoin> joins = pendingJoins.iterator();
		while (joins.hasNext()) {
			PendingJoin join = joins.next();
			if (!join.ok().isDone()) continue;
			joins.remove();
			step(() -> bridge.completeJoin(engine, join.id(), join.ok().join()));
		}
	}

	private void fail(YakVcException e) {
		YakVcClient.LOGGER.error("Native call failed", e);
		if (e.poisoned()) {
			YakVcClient.LOGGER.error("Voice disabled until restart: the native engine crashed");
			VoiceToasts.show(Component.translatable("yakvc.toast.crashed"));
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

	public ClientConfig config() {
		return config;
	}

	public VoiceKeys keys() {
		return keys;
	}

	/** Saved per-player volume and mute. Changes reach the engine on the next tick. */
	public PlayerVolumes volumes() {
		return volumes;
	}

	/** Muted because the player is blocked in Social Interactions. */
	public boolean blocked(UUID uuid) {
		return blocked.contains(uuid);
	}

	public EngineEvent.@Nullable RendezvousState rendezvous() {
		return rendezvous;
	}

	/** The connection state of a peer the engine knows, or null. */
	public EngineEvent.@Nullable PeerState peerState(UUID uuid) {
		return peers.get(uuid);
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
