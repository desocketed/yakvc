package io.github.desocketed.yakvc;

import io.github.desocketed.yakvc.config.ClientConfig;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.client.multiplayer.chat.ChatRestriction;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;

/**
 * Whether voice may run on the current server connection: from {@code ClientPlayConnectionEvents.JOIN} to
 * DISCONNECT.
 *
 * <p>Server owners opt out with {@value #OPT_OUT_MARKER} in their MOTD. The MOTD of a direct connect only arrives
 * in the server-data packet after join, so voice waits for that packet (at most 5 s) before starting.
 */
public final class VoiceSession {
	public static final String OPT_OUT_MARKER = "[no-yakvc]";
	private static final int SERVER_DATA_TIMEOUT_TICKS = 5 * 20;

	public final ClientPacketListener connection;
	private final @Nullable ServerData serverData;
	/** The MOTD object at join. The server-data packet replaces it, so a different object means it arrived. */
	private final @Nullable Component motdAtJoin;
	private int ticksWaited;
	private boolean waiting;
	private @Nullable String offReason;

	public VoiceSession(ClientPacketListener connection, Minecraft minecraft, ClientConfig config) {
		this.connection = connection;
		this.serverData = connection.getServerData();
		this.motdAtJoin = serverData == null ? null : serverData.motd;
		// Singleplayer has no server data and no MOTD to wait for.
		this.waiting = serverData != null;
		if (config.respectChatRestrictions() && chatDisabled(minecraft)) {
			offReason = "chat is disabled for this account";
			YakVcClient.LOGGER.info("Voice off: {}", offReason);
		}
	}

	/** Called every client tick. */
	public void tick() {
		if (offReason != null) return;
		if (serverData != null && serverData.motd != null && serverData.motd.getString().contains(OPT_OUT_MARKER)) {
			offReason = "the server opted out with " + OPT_OUT_MARKER;
			YakVcClient.LOGGER.info("Voice off: {}", offReason);
			return;
		}
		if (waiting && serverData.motd != motdAtJoin) {
			waiting = false;
		} else if (waiting && ++ticksWaited >= SERVER_DATA_TIMEOUT_TICKS) {
			YakVcClient.LOGGER.info("No server data after 5 s; starting voice anyway");
			waiting = false;
		}
	}

	/** Voice may run now. */
	public boolean active() {
		return offReason == null && !waiting;
	}

	/** Why voice is off for the whole connection, or null. */
	public @Nullable String offReason() {
		return offReason;
	}

	private static boolean chatDisabled(Minecraft minecraft) {
		return minecraft.computeChatAbilities().restrictions()
				.anyMatch(r -> r == ChatRestriction.DISABLED_BY_PROFILE || r == ChatRestriction.DISABLED_BY_LAUNCHER);
	}
}
