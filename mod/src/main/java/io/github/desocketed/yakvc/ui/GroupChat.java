package io.github.desocketed.yakvc.ui;

import com.mojang.brigadier.arguments.StringArgumentType;
import com.mojang.brigadier.context.CommandContext;
import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.input.LookTarget;
import io.github.desocketed.yakvc.natives.EngineEvent;
import io.github.desocketed.yakvc.natives.VoiceGroup;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.Locale;
import java.util.Map;
import java.util.UUID;
import net.fabricmc.fabric.api.client.command.v2.ClientCommandRegistrationCallback;
import net.fabricmc.fabric.api.client.command.v2.ClientCommands;
import net.fabricmc.fabric.api.client.command.v2.FabricClientCommandSource;
import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.player.AbstractClientPlayer;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.commands.SharedSuggestionProvider;
import net.minecraft.network.chat.ClickEvent;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.ComponentUtils;
import net.minecraft.network.chat.HoverEvent;
import net.minecraft.world.phys.AABB;
import org.jspecify.annotations.Nullable;

/**
 * The player's side of groups outside the groups screen: the Group key, invites in chat, other members' changes as
 * toasts, and kick votes in chat with the {@code /yakvc kick} client command (DESIGN.md "Group voice chat"). Runs on
 * the client thread.
 */
public final class GroupChat {
	/** As long as the engine keeps an invite. */
	private static final long INVITE_NANOS = 120_000_000_000L;
	/** As long as the engine keeps a kick vote open. */
	private static final long VOTE_NANOS = 60_000_000_000L;

	private final GameStateFeeder feeder;
	/** Invites to us not accepted yet, by who sent them, with when they came. */
	private final Map<UUID, Long> invites = new HashMap<>();
	/** Kick votes we have seen start, by target, with when, so only the first ballot asks us to vote. */
	private final Map<UUID, Long> votes = new HashMap<>();

	public GroupChat(GameStateFeeder feeder) {
		this.feeder = feeder;
	}

	/** Forgets invites and votes, which belong to one server. */
	public void clear() {
		invites.clear();
		votes.clear();
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
			case KICKED_US -> "yakvc.group.notice.kicked_us";
			case MADE_PUBLIC -> "yakvc.group.notice.made_public";
			case MADE_PRIVATE -> "yakvc.group.notice.made_private";
			case LABEL_CHANGED -> "yakvc.group.notice.label_changed";
		};
		UUID who = switch (notice.kind()) {
			case JOINED, LEFT, KICKED_US -> notice.uuid();
			case MADE_PUBLIC, MADE_PRIVATE, LABEL_CHANGED -> notice.by();
		};
		if (notice.kind() == EngineEvent.NoticeKind.LEFT) votes.remove(notice.uuid());
		if (notice.kind() == EngineEvent.NoticeKind.KICKED_US) votes.clear();
		VoiceToasts.showGroupNotice(Component.translatable(key, feeder.name(who)));
	}

	/**
	 * A ballot on kicking a member. The first one we see of a vote asks us to vote too, with clickable [Yes] and [No]
	 * (unless it is our own or the vote is about us); later ones show how far the vote is.
	 */
	public void kickVote(EngineEvent.KickVote vote) {
		long now = System.nanoTime();
		votes.values().removeIf(at -> now - at >= VOTE_NANOS);
		boolean started = votes.putIfAbsent(vote.target(), now) == null;
		UUID me = me();
		boolean aboutUs = vote.target().equals(me);
		String target = feeder.name(vote.target());
		if (started && vote.yes() && !vote.by().equals(me)) {
			String by = feeder.name(vote.by());
			chat(aboutUs ? Component.translatable("yakvc.kick.started_us", by)
					: Component.translatable("yakvc.kick.started", by, target).append(" ")
							.append(voteLink(target, true)).append(" ").append(voteLink(target, false)));
			return;
		}
		chat(aboutUs ? Component.translatable("yakvc.kick.progress_us", vote.votes(), vote.needed())
				: Component.translatable("yakvc.kick.progress", target, vote.votes(), vote.needed()));
	}

	/**
	 * [Yes] or [No], running {@code /yakvc kick}. Clicked chat commands go through the same path as typed ones, where
	 * Fabric runs client commands before anything reaches the server.
	 */
	private static Component voteLink(String target, boolean yes) {
		String command = "yakvc kick " + target + (yes ? " yes" : " no");
		Component hover = Component.translatable(yes ? "yakvc.kick.yes.hover" : "yakvc.kick.no.hover", target);
		return ComponentUtils.wrapInSquareBrackets(Component.translatable(yes ? "yakvc.kick.yes" : "yakvc.kick.no"))
				.withStyle(style -> style.withColor(yes ? ChatFormatting.GREEN : ChatFormatting.RED)
						.withClickEvent(new ClickEvent.RunCommand(command))
						.withHoverEvent(new HoverEvent.ShowText(hover)));
	}

	/** Registers {@code /yakvc kick <player> [yes|no]}, which votes yes unless told no. */
	public void registerCommand() {
		ClientCommandRegistrationCallback.EVENT.register((dispatcher, context) -> dispatcher.register(
				ClientCommands.literal("yakvc").then(ClientCommands.literal("kick").then(
						ClientCommands.argument("player", StringArgumentType.word())
								.suggests((c, builder) -> SharedSuggestionProvider.suggest(mates().keySet(), builder))
								.executes(c -> kick(c, true))
								.then(ClientCommands.literal("yes").executes(c -> kick(c, true)))
								.then(ClientCommands.literal("no").executes(c -> kick(c, false)))))));
	}

	private int kick(CommandContext<FabricClientCommandSource> context, boolean yes) {
		String name = StringArgumentType.getString(context, "player");
		UUID target = null;
		for (Map.Entry<String, UUID> mate : mates().entrySet()) {
			if (mate.getKey().toLowerCase(Locale.ROOT).equals(name.toLowerCase(Locale.ROOT))) target = mate.getValue();
		}
		if (target == null) {
			context.getSource().sendError(Component.translatable("yakvc.kick.not_a_member", name));
			return 0;
		}
		if (!feeder.voteKick(target, yes)) {
			context.getSource().sendError(Component.translatable("yakvc.kick.not_a_member", name));
			return 0;
		}
		// The engine reports our own ballot too, which shows the vote's progress.
		return 1;
	}

	/** The other members of our group, by name. */
	private Map<String, UUID> mates() {
		Map<String, UUID> mates = new LinkedHashMap<>();
		VoiceGroup mine = feeder.myGroup();
		if (mine == null) return mates;
		for (UUID member : mine.members()) {
			if (!member.equals(me())) mates.put(feeder.name(member), member);
		}
		return mates;
	}

	private static @Nullable UUID me() {
		LocalPlayer player = Minecraft.getInstance().player;
		return player == null ? null : player.getUUID();
	}

	private static void chat(Component message) {
		LocalPlayer player = Minecraft.getInstance().player;
		if (player != null) player.sendSystemMessage(message);
	}
}
