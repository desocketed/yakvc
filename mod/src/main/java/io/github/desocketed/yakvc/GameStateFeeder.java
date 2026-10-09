package io.github.desocketed.yakvc;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.mojang.authlib.GameProfile;
import io.github.desocketed.yakvc.config.ClientConfig;
import io.github.desocketed.yakvc.config.PlayerVolumes;
import io.github.desocketed.yakvc.input.VoiceKeys;
import io.github.desocketed.yakvc.natives.EngineEvent;
import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.natives.VoiceGroup;
import io.github.desocketed.yakvc.natives.VoiceStats;
import io.github.desocketed.yakvc.natives.YakVcException;
import io.github.desocketed.yakvc.ui.EngineErrorToasts;
import io.github.desocketed.yakvc.ui.GroupChat;
import io.github.desocketed.yakvc.ui.VoiceMenuScreen;
import io.github.desocketed.yakvc.ui.VoiceToasts;
import java.lang.foreign.MemorySegment;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Iterator;
import java.util.LinkedHashMap;
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
	/** How often the debug overlay's snapshot is refreshed: four times a second. */
	private static final int STATS_REFRESH_TICKS = 5;
	/** The engine's level for a silent frame. */
	public static final float SILENCE_DB = -100;
	/** The longest group label the engine takes. */
	public static final int MAX_LABEL_CHARS = 32;

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

	private record Identity(UUID uuid, String name) {}

	// What the engine was last told, so unchanged values aren't sent again.
	private @Nullable Identity identity;
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
	/** The rendezvous event's value: ticket expiry (Unix seconds) when registered, retry delay (ms) when retrying. */
	private long rendezvousValue;
	/** Registered with a verified ticket. */
	private boolean verified;
	private final Map<UUID, EngineEvent.PeerState> peers = new HashMap<>();
	private final Set<UUID> verifiedPeers = new HashSet<>();
	private final Set<UUID> talking = new LinkedHashSet<>();
	private int errorEvents;
	private final EngineErrorToasts errorToasts = new EngineErrorToasts();
	private boolean micHeard;
	/** The last microphone level and when it came, for the meter in the settings. */
	private float micLevelDb = SILENCE_DB;
	private long micLevelNanos;
	/** Whether the debug overlay shows, and its latest snapshot (null until the first refresh). */
	private boolean debugOverlay;
	private @Nullable VoiceStats stats;
	private int ticksUntilStats;
	/** Whether the last tick had a server session, to leave our group once it ends. */
	private boolean hadSession;
	/** Invites, group changes and kick votes, told in chat and toasts. */
	private final GroupChat groupChat = new GroupChat(this);

	public GameStateFeeder(NativeBridge bridge, MemorySegment engine, ClientConfig config, VoiceKeys keys,
			SessionJoiner joiner, PlayerVolumes volumes) {
		this.bridge = bridge;
		this.engine = engine;
		this.config = config;
		this.keys = keys;
		this.joiner = joiner;
		this.volumes = volumes;
	}

	public void join(ClientPacketListener connection, Minecraft minecraft) {
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
		step(this::refreshStats);
	}

	/** Polls the engine's snapshot a few times a second, and only while the overlay shows it. */
	private void refreshStats() {
		if (!debugOverlay || --ticksUntilStats > 0) return;
		ticksUntilStats = STATS_REFRESH_TICKS;
		stats = VoiceStats.parse(bridge.stats(engine));
	}

	private void feedGame(Minecraft minecraft) {
		keys.tick();
		if (keys.debugOverlayPressed()) setDebugOverlay(!debugOverlay);
		if (keys.menuPressed() && minecraft.gui.screen() == null) {
			minecraft.gui.setScreen(new VoiceMenuScreen(null, this));
		}
		VoiceSession session = this.session;
		step(() -> feedIdentity(minecraft, session));
		if (session != null) session.tick();
		else talking.clear();
		// Groups last one server session.
		if (session == null && hadSession) {
			step(this::leaveGroup);
			groupChat.clear();
		}
		hadSession = session != null;
		LocalPlayer player = minecraft.player;
		boolean active = session != null && session.active() && player != null && minecraft.level != null;
		if (keys.groupPressed() && active) step(() -> groupChat.groupKeyPressed(minecraft));

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

	/**
	 * The audio devices, inputs or outputs, as id to name in the system's order. The id goes in the config, the name
	 * is for the player. Empty if the system can't list them.
	 */
	public Map<String, String> devices(boolean inputs) {
		if (closed) return Map.of();
		Map<String, String> devices = new LinkedHashMap<>();
		try {
			JsonElement listed = JsonParser.parseString(bridge.listDevices(engine));
			for (JsonElement device : listed.getAsJsonObject().getAsJsonArray(inputs ? "inputs" : "outputs")) {
				JsonObject fields = device.getAsJsonObject();
				devices.put(fields.get("id").getAsString(), fields.get("name").getAsString());
			}
		} catch (YakVcException e) {
			if (e.poisoned()) fail(e);
			else YakVcClient.LOGGER.warn("Could not list audio devices: {}", e.getMessage());
		}
		return devices;
	}

	/** Our group first, if we are in one, then the public groups our peers announce. Empty if the engine is stopped. */
	public List<VoiceGroup> groups() {
		if (closed) return List.of();
		try {
			return VoiceGroup.parse(bridge.listGroups(engine));
		} catch (YakVcException e) {
			fail(e);
			return List.of();
		}
	}

	/** Our own group, or null if we are in none. */
	public @Nullable VoiceGroup myGroup() {
		List<VoiceGroup> groups = groups();
		return groups.isEmpty() || !groups.getFirst().mine() ? null : groups.getFirst();
	}

	/** Invites a player to our group, or to a new one if we are in none. False if the engine is stopped. */
	public boolean invite(UUID target) {
		return groupCall(() -> bridge.invite(engine, target));
	}

	/** Joins the group of {@code from}'s invite. False if there was none in the last two minutes. */
	public boolean acceptInvite(UUID from) {
		return groupCall(() -> bridge.acceptInvite(engine, from));
	}

	/** Joins a public group from {@link #groups}. False if nobody announces it any more. */
	public boolean joinGroup(String id) {
		return groupCall(() -> bridge.joinGroup(engine, id));
	}

	public void leaveGroup() {
		groupCall(() -> bridge.leaveGroup(engine));
	}

	/** False if we are in no group. */
	public boolean setGroupPublic(boolean isPublic) {
		return groupCall(() -> bridge.setGroupPublic(engine, isPublic));
	}

	/** Sets our group's label: empty for none, at most {@link #MAX_LABEL_CHARS}. False if refused. */
	public boolean setGroupLabel(String label) {
		return groupCall(() -> bridge.setGroupLabel(engine, label));
	}

	/** Votes on kicking a group mate. False if we are in no group or {@code target} is not in it. */
	public boolean voteKick(UUID target, boolean yes) {
		return groupCall(() -> bridge.voteKick(engine, target, yes));
	}

	/**
	 * Runs a group action and returns whether it worked. A refusal (no such invite or group any more) is a normal
	 * outcome the caller tells the player about, so it is only logged.
	 */
	private boolean groupCall(Runnable call) {
		if (closed) return false;
		try {
			call.run();
			return true;
		} catch (YakVcException e) {
			if (e.poisoned()) fail(e);
			else YakVcClient.LOGGER.info("Group action refused: {}", e.getMessage());
			return false;
		}
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

	/**
	 * Sets who we are, which starts rendezvous authentication, from the title screen on. On an offline-mode server the
	 * connection's profile differs from the launcher's account and the tab list uses it; back outside a server the
	 * account is restored, so the next online join doesn't have to re-authenticate first.
	 */
	private void feedIdentity(Minecraft minecraft, @Nullable VoiceSession session) {
		Identity wanted;
		if (session != null) {
			GameProfile profile = session.connection.getLocalGameProfile();
			wanted = new Identity(profile.id(), profile.name());
		} else {
			wanted = new Identity(minecraft.getUser().getProfileId(), minecraft.getUser().getName());
		}
		if (wanted.equals(identity)) return;
		identity = wanted;
		bridge.setIdentity(engine, wanted.uuid(), wanted.name());
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

	/** Acts on one engine event. Public for the client gametest, which feeds it errors; must run on the client thread. */
	public void handle(EngineEvent event) {
		switch (event) {
			case EngineEvent.JoinRequest(int id, String serverId) ->
					pendingJoins.add(new PendingJoin(id, joiner.joinAsync(serverId)));
			case EngineEvent.Rendezvous(EngineEvent.RendezvousState state, long value, boolean isVerified) -> {
				rendezvous = state;
				rendezvousValue = value;
				verified = state == EngineEvent.RendezvousState.REGISTERED && isVerified;
				YakVcClient.LOGGER.info("Rendezvous {} ({}){}", state, value, verified ? " verified" : "");
			}
			case EngineEvent.Peer(UUID uuid, EngineEvent.PeerState state, boolean isVerified) -> {
				if (state == EngineEvent.PeerState.GONE) {
					talking.remove(uuid);
					peers.remove(uuid);
					verifiedPeers.remove(uuid);
				} else {
					peers.put(uuid, state);
					if (isVerified) verifiedPeers.add(uuid);
					else verifiedPeers.remove(uuid);
				}
				YakVcClient.LOGGER.info("Peer {} {} {}{}", uuid, name(uuid), state, isVerified ? " verified" : "");
			}
			case EngineEvent.Talking(UUID uuid, boolean isTalking) -> {
				if (isTalking) talking.add(uuid);
				else talking.remove(uuid);
				// DEBUG, because every talk spurt changes it and a long conversation would flood latest.log.
				YakVcClient.LOGGER.debug("Talking {} {} {}", uuid, name(uuid), isTalking);
			}
			case EngineEvent.MicLevel(float db) -> {
				// Levels only arrive while the microphone (or the dev test tone) delivers audio, so the first one
				// shows in the log that capture is working.
				micLevelDb = db;
				micLevelNanos = System.nanoTime();
				if (!micHeard) {
					micHeard = true;
					YakVcClient.LOGGER.info("Microphone is delivering audio ({} dBFS)", db);
				}
			}
			case EngineEvent.Error(String message) -> {
				errorEvents++;
				YakVcClient.LOGGER.warn("Engine: {}", message);
				errorToasts.show(message);
			}
			case EngineEvent.Invite invite -> {
				YakVcClient.LOGGER.info("Group invite from {} {}", invite.from(), name(invite.from()));
				groupChat.invited(invite.from());
			}
			case EngineEvent.GroupNotice notice -> {
				YakVcClient.LOGGER.info("Group {}: {} {}, by {}", notice.kind(), notice.uuid(), name(notice.uuid()),
						notice.by());
				groupChat.notice(notice);
			}
			case EngineEvent.KickVote vote -> {
				YakVcClient.LOGGER.info("Kick vote on {} by {}: {} ({} of {})", vote.target(), vote.by(),
						vote.yes() ? "yes" : "no", vote.votes(), vote.needed());
				groupChat.kickVote(vote);
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

	public GroupChat groupChat() {
		return groupChat;
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

	/** Ticket expiry in Unix seconds when registered, the retry delay in ms when retrying, else 0. */
	public long rendezvousValue() {
		return rendezvousValue;
	}

	/** Whether we are registered with a verified ticket, i.e. our Mojang account is proven. */
	public boolean verified() {
		return verified;
	}

	/** The connection state of a peer the engine knows, or null. */
	public EngineEvent.@Nullable PeerState peerState(UUID uuid) {
		return peers.get(uuid);
	}

	/** Whether a peer proved its Mojang account. */
	public boolean peerVerified(UUID uuid) {
		return verifiedPeers.contains(uuid);
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

	public boolean debugOverlay() {
		return debugOverlay;
	}

	/** Shows or hides the debug overlay. Not saved: it is for a test or a bug report, not for every game. */
	public void setDebugOverlay(boolean on) {
		debugOverlay = on;
		ticksUntilStats = 0;
		if (!on) stats = null;
	}

	/** The debug overlay's latest engine snapshot, or null while it is off or not fetched yet. */
	public @Nullable VoiceStats stats() {
		return stats;
	}

	/** Engine {@code Error} events so far. */
	public int errorEvents() {
		return errorEvents;
	}

	/**
	 * The microphone's level in dBFS. The engine reports it about ten times a second whether or not we transmit, so a
	 * level that stopped coming means no working microphone, and reads as silence.
	 */
	public float micLevelDb() {
		boolean stale = System.nanoTime() - micLevelNanos > 500_000_000L;
		return stale ? SILENCE_DB : micLevelDb;
	}

	/**
	 * Whether the microphone delivered audio in the last second. Levels come about ten times a second while it does,
	 * muted or not, so none for a second means it is missing, busy or failed.
	 */
	public boolean micWorking() {
		return micHeard && System.nanoTime() - micLevelNanos < 1_000_000_000L;
	}
}
