package io.github.desocketed.yakvc.ui;

import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.input.LookTarget;
import io.github.desocketed.yakvc.natives.EngineEvent;
import io.github.desocketed.yakvc.natives.VoiceGroup;
import java.util.HashMap;
import java.util.Map;
import java.util.UUID;
import net.minecraft.client.Minecraft;
import net.minecraft.client.player.AbstractClientPlayer;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.network.chat.Component;
import net.minecraft.world.phys.AABB;

/**
 * The player's side of groups outside the groups screen: the Group key, invites in chat and other members' changes as
 * toasts (DESIGN.md "Group voice chat"). Runs on the client thread.
 */
public final class GroupChat {
	/** As long as the engine keeps an invite. */
	private static final long INVITE_NANOS = 120_000_000_000L;

	private final GameStateFeeder feeder;
	/** Invites to us not accepted yet, by who sent them, with when they came. */
	private final Map<UUID, Long> invites = new HashMap<>();

	public GroupChat(GameStateFeeder feeder) {
		this.feeder = feeder;
	}

	/** Forgets invites, which belong to one server. */
	public void clear() {
		invites.clear();
	}

	/**
	 * The Group key: accepts the invite of the player looked at, if they sent one, and otherwise invites them. A group
	 * is made face to face, so only players in view count.
	 */
	public void groupKeyPressed(Minecraft minecraft) {
		LocalPlayer player = minecraft.player;
		if (player == null || minecraft.level == null) return;
		Map<UUID, AABB> inView = new HashMap<>();
		for (AbstractClientPlayer other : minecraft.level.players()) {
			if (other == player || other.isSpectator() || !player.hasLineOfSight(other)) continue;
			inView.put(other.getUUID(), other.getBoundingBox());
		}
		UUID target = LookTarget.pick(player.getEyePosition(), player.getViewVector(1), inView);
		if (target == null) {
			player.sendOverlayMessage(Component.translatable("yakvc.group.no_target"));
			return;
		}
		String name = feeder.name(target);
		VoiceGroup mine = feeder.myGroup();
		if (mine != null && mine.members().contains(target)) {
			player.sendOverlayMessage(Component.translatable("yakvc.group.already_in", name));
			return;
		}
		if (invites.remove(target) != null && feeder.acceptInvite(target)) {
			player.sendSystemMessage(Component.translatable("yakvc.group.joined", name));
			return;
		}
		// An invite that just expired falls through to inviting them back.
		if (!connected(target)) {
			player.sendOverlayMessage(Component.translatable("yakvc.group.no_voice", name));
			return;
		}
		if (feeder.invite(target)) player.sendSystemMessage(Component.translatable("yakvc.group.invited", name));
	}

	/** An invite goes only over a voice connection, so without one it would be lost without a word. */
	private boolean connected(UUID uuid) {
		EngineEvent.PeerState state = feeder.peerState(uuid);
		return state == EngineEvent.PeerState.DIRECT || state == EngineEvent.PeerState.RELAYED
				|| state == EngineEvent.PeerState.RELAY_FULL;
	}

	/** {@code from} invited us: says so in chat, with the key that accepts. */
	public void invited(UUID from) {
		long now = System.nanoTime();
		invites.values().removeIf(at -> now - at >= INVITE_NANOS);
		invites.put(from, now);
		chat(Component.translatable("yakvc.group.invite", feeder.name(from),
				feeder.keys().group.getTranslatedKeyMessage()));
	}

	/** Whether {@code from} sent us an invite we haven't accepted. For tests. */
	public boolean invitedBy(UUID from) {
		return invites.containsKey(from);
	}

	/** A change another player made to our group, as a toast. */
	public void notice(EngineEvent.GroupNotice notice) {
		String key = switch (notice.kind()) {
			case JOINED -> "yakvc.group.notice.joined";
			case LEFT -> "yakvc.group.notice.left";
			case MADE_PUBLIC -> "yakvc.group.notice.made_public";
			case MADE_PRIVATE -> "yakvc.group.notice.made_private";
			case LABEL_CHANGED -> "yakvc.group.notice.label_changed";
		};
		UUID who = switch (notice.kind()) {
			case JOINED, LEFT -> notice.uuid();
			case MADE_PUBLIC, MADE_PRIVATE, LABEL_CHANGED -> notice.by();
		};
		VoiceToasts.showGroupNotice(Component.translatable(key, feeder.name(who)));
	}

	private static void chat(Component message) {
		LocalPlayer player = Minecraft.getInstance().player;
		if (player != null) player.sendSystemMessage(message);
	}
}

