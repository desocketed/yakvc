package io.github.desocketed.yakvc.ui;

import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.VoiceSession;
import io.github.desocketed.yakvc.YakVcClient;
import io.github.desocketed.yakvc.config.ClientConfig;
import io.github.desocketed.yakvc.config.PlayerVolumes;
import io.github.desocketed.yakvc.input.VoiceKeys;
import io.github.desocketed.yakvc.natives.EngineEvent;
import java.io.IOException;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Locale;
import java.util.UUID;
import java.util.function.Consumer;
import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.AbstractSliderButton;
import net.minecraft.client.gui.components.AbstractWidget;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.ContainerObjectSelectionList;
import net.minecraft.client.gui.components.PlayerFaceExtractor;
import net.minecraft.client.gui.components.StringWidget;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.layouts.HeaderAndFooterLayout;
import net.minecraft.client.gui.layouts.LinearLayout;
import net.minecraft.client.gui.narration.NarratableEntry;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.network.chat.CommonComponents;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;

/**
 * The voice menu: voice status, mute and deafen, every other player on the server with their voice connection and a
 * volume slider and mute button, saved by UUID, and the way to the settings screen.
 */
public final class VoiceMenuScreen extends Screen {
	private static final int ROW_HEIGHT = 28;
	private static final int ROW_WIDTH = 310;
	private static final int SETUP_HEIGHT = 58;
	private static final int HEADER_HEIGHT = 45;

	private final @Nullable Screen lastScreen;
	private final GameStateFeeder feeder;
	private final HeaderAndFooterLayout layout = new HeaderAndFooterLayout(this, HEADER_HEIGHT, 33);
	private @Nullable PlayerList list;
	/** The players in the list, to rebuild it when someone joins or leaves. */
	private List<UUID> shown = List.of();
	/** The status line under the title, which follows the voice state while the menu is open. */
	private @Nullable StringWidget statusText;
	private Component shownStatus = Component.empty();

	public VoiceMenuScreen(@Nullable Screen lastScreen, GameStateFeeder feeder) {
		super(Component.translatable("yakvc.menu.title"));
		this.lastScreen = lastScreen;
		this.feeder = feeder;
	}

	@Override
	protected void init() {
		// init runs again on every rebuild, and the layout would keep the old widgets.
		layout.removeChildren();
		LinearLayout header = layout.addToHeader(LinearLayout.vertical().spacing(4));
		header.defaultCellSetting().alignHorizontallyCenter();
		header.addChild(new StringWidget(title, font));
		shownStatus = status();
		statusText = header.addChild(new StringWidget(shownStatus.copy().withStyle(ChatFormatting.GRAY), font));

		list = layout.addToContents(new PlayerList(minecraft));
		// The first-run card sits at the top of the list, so it scrolls away with the players.
		if (!feeder.config().setupDone()) list.add(new SetupRow(), SETUP_HEIGHT);
		shown = players();
		for (UUID uuid : shown) list.add(new PlayerRow(uuid), ROW_HEIGHT);

		VoiceKeys keys = feeder.keys();
		LinearLayout footer = layout.addToFooter(LinearLayout.horizontal().spacing(4));
		footer.addChild(Button.builder(muteLabel(keys.muted()), button -> {
			keys.setMuted(!keys.muted());
			button.setMessage(muteLabel(keys.muted()));
		}).width(60).build());
		footer.addChild(Button.builder(deafenLabel(keys.deafened()), button -> {
			keys.setDeafened(!keys.deafened());
			button.setMessage(deafenLabel(keys.deafened()));
		}).width(60).build());
		footer.addChild(Button.builder(Component.translatable("yakvc.menu.groups"),
				button -> minecraft.gui.setScreen(new GroupsScreen(this, feeder))).width(60).build());
		footer.addChild(Button.builder(Component.translatable("yakvc.menu.settings"),
				button -> minecraft.gui.setScreen(new VoiceSettingsScreen(this, minecraft.options, feeder))).width(60).build());
		footer.addChild(Button.builder(CommonComponents.GUI_DONE, button -> onClose()).width(60).build());

		layout.visitWidgets(this::addRenderableWidget);
		repositionElements();
	}

	@Override
	protected void repositionElements() {
		layout.arrangeElements();
		if (list != null) list.updateSize(width, layout);
	}

	@Override
	public void tick() {
		if (!players().equals(shown)) {
			rebuildWidgets();
			return;
		}
		Component status = status();
		if (statusText != null && !status.equals(shownStatus)) {
			shownStatus = status;
			statusText.setMessage(status.copy().withStyle(ChatFormatting.GRAY));
		}
	}

	@Override
	public void removed() {
		try {
			feeder.volumes().save();
		} catch (IOException e) {
			YakVcClient.LOGGER.error("Could not save player volumes", e);
		}
	}

	@Override
	public void onClose() {
		minecraft.gui.setScreen(lastScreen);
	}

	/** Everyone else on the server: voice peers first, then by name. */
	private List<UUID> players() {
		VoiceSession session = feeder.session();
		if (session == null) return List.of();
		List<UUID> players = new ArrayList<>(session.connection.getOnlinePlayerIds());
		if (minecraft.player != null) players.remove(minecraft.player.getUUID());
		players.sort(Comparator.comparing((UUID uuid) -> feeder.peerState(uuid) == null)
				.thenComparing(uuid -> feeder.name(uuid).toLowerCase(Locale.ROOT)));
		return players;
	}

	private Component status() {
		Component problem = VoiceHud.problem(feeder);
		if (problem != null) return problem;
		if (feeder.session() == null) return Component.translatable("yakvc.menu.status.no_world");
		EngineEvent.RendezvousState rendezvous = feeder.rendezvous();
		return rendezvous == EngineEvent.RendezvousState.REGISTERED
				? Component.translatable("yakvc.menu.status.ready")
				: Component.translatable("yakvc.menu.status.connecting");
	}

	private static Component muteLabel(boolean muted) {
		return Component.translatable(muted ? "yakvc.menu.unmute_mic" : "yakvc.menu.mute_mic");
	}

	private static Component deafenLabel(boolean deafened) {
		return Component.translatable(deafened ? "yakvc.menu.undeafen" : "yakvc.menu.deafen");
	}

	private final class PlayerList extends ContainerObjectSelectionList<Row> {
		PlayerList(Minecraft minecraft) {
			super(minecraft, VoiceMenuScreen.this.width, layout.getContentHeight(), layout.getHeaderHeight(), ROW_HEIGHT);
		}

		void add(Row row, int height) {
			addEntry(row, height);
		}

		@Override
		public int getRowWidth() {
			return ROW_WIDTH;
		}
	}

	/** A row of the list: the first-run card or a player. */
	private abstract static class Row extends ContainerObjectSelectionList.Entry<Row> {
	}

	/**
	 * The first-run card: who may invite you to a group and how you talk, the two choices a new player should make
	 * before anything else. Got it saves both and sets {@code setup_done}, so the card doesn't come back.
	 */
	private final class SetupRow extends Row {
		private final AbstractWidget invites;
		private final AbstractWidget activation;
		private final Button gotIt;

		SetupRow() {
			ClientConfig config = feeder.config();
			OptionInstance<String> invitesOption = VoiceSettingsScreen.invitesOption(config);
			OptionInstance<String> activationOption = VoiceSettingsScreen.activationOption(config);
			invites = invitesOption.createButton(minecraft.options, 0, 0, 125);
			activation = activationOption.createButton(minecraft.options, 0, 0, 125);
			gotIt = Button.builder(Component.translatable("yakvc.setup.ok"), button -> {
				ClientConfig changed = VoiceSettingsScreen.withActivationAndInvites(config, activationOption, invitesOption)
						.with("", "setup_done", "true");
				VoiceSettingsScreen.applyAndSave(feeder, changed);
				rebuildWidgets();
			}).width(52).build();
		}

		@Override
		public void extractContent(GuiGraphicsExtractor graphics, int mouseX, int mouseY, boolean hovered, float a) {
			int x = getContentX();
			int y = getContentY();
			int width = getContentWidth();
			graphics.outline(x - 4, y, width + 8, SETUP_HEIGHT - 4, 0xFFA0A0A0);
			Component prompt = Component.translatable("yakvc.setup.prompt");
			graphics.text(font, prompt, x + (width - font.width(prompt)) / 2, y + 5, 0xFFFFFFFF, false);
			invites.setPosition(x, y + 17);
			activation.setPosition(invites.getRight() + 4, y + 17);
			gotIt.setPosition(activation.getRight() + 4, y + 17);
			invites.extractRenderState(graphics, mouseX, mouseY, a);
			activation.extractRenderState(graphics, mouseX, mouseY, a);
			gotIt.extractRenderState(graphics, mouseX, mouseY, a);
			Component later = Component.translatable("yakvc.setup.later");
			graphics.text(font, later, x + (width - font.width(later)) / 2, y + 41, 0xFFA0A0A0, false);
		}

		@Override
		public List<? extends GuiEventListener> children() {
			return List.of(invites, activation, gotIt);
		}

		@Override
		public List<? extends NarratableEntry> narratables() {
			return List.of(invites, activation, gotIt);
		}
	}

	/** Face, name, verified badge, talking icon and voice connection on the left; volume and mute on the right. */
	private final class PlayerRow extends Row {
		private final UUID uuid;
		private final VolumeSlider volume;
		private final Button mute;

		PlayerRow(UUID uuid) {
			this.uuid = uuid;
			PlayerVolumes.Setting setting = feeder.volumes().get(uuid);
			volume = new VolumeSlider(setting.volume(), v -> feeder.volumes().set(uuid,
					new PlayerVolumes.Setting(v, feeder.volumes().get(uuid).muted())));
			mute = Button.builder(playerMuteLabel(setting.muted()), button -> {
				PlayerVolumes.Setting now = feeder.volumes().get(uuid);
				feeder.volumes().set(uuid, new PlayerVolumes.Setting(now.volume(), !now.muted()));
				button.setMessage(playerMuteLabel(!now.muted()));
			}).width(56).build();
		}

		@Override
		public void extractContent(GuiGraphicsExtractor graphics, int mouseX, int mouseY, boolean hovered, float a) {
			int x = getContentX() + 2;
			int y = getContentY();
			VoiceSession session = feeder.session();
			PlayerInfo info = session == null ? null : session.connection.getPlayerInfo(uuid);
			if (info != null) PlayerFaceExtractor.extractRenderState(graphics, info.getSkin(), x, y + 2, 20);

			int textX = x + 24;
			String name = feeder.name(uuid);
			graphics.text(font, name, textX, y + 3, 0xFFFFFFFF, false);
			int iconX = textX + font.width(name) + 4;
			if (feeder.peerVerified(uuid)) {
				graphics.text(font, Icons.VERIFIED, iconX, y + 3, 0xFFFFFFFF, false);
				iconX += font.width(Icons.VERIFIED) + 3;
			}
			if (feeder.talking().contains(uuid)) {
				graphics.text(font, Icons.SPEAKER, iconX, y + 3, 0xFFFFFFFF, false);
			}
			Component connection = connection();
			graphics.text(font, connection, textX, y + 14, 0xFFA0A0A0, false);
			Component mismatch = VoiceToasts.versionMismatch(feeder.peerState(uuid), name);
			if (mismatch != null && mouseX >= textX && mouseX < textX + font.width(connection)
					&& mouseY >= y + 14 && mouseY < y + 14 + font.lineHeight) {
				graphics.setTooltipForNextFrame(mismatch, mouseX, mouseY);
			}

			mute.setPosition(getContentRight() - mute.getWidth(), y + 2);
			volume.setPosition(mute.getX() - 4 - volume.getWidth(), y + 2);
			volume.active = !feeder.blocked(uuid);
			mute.active = !feeder.blocked(uuid);
			volume.extractRenderState(graphics, mouseX, mouseY, a);
			mute.extractRenderState(graphics, mouseX, mouseY, a);
		}

		private Component connection() {
			if (feeder.blocked(uuid)) return Component.translatable("yakvc.menu.peer.blocked");
			EngineEvent.PeerState state = feeder.peerState(uuid);
			String key = state == null ? "none" : state.name().toLowerCase(Locale.ROOT);
			return Component.translatable("yakvc.menu.peer." + key);
		}

		@Override
		public List<? extends GuiEventListener> children() {
			return List.of(volume, mute);
		}

		@Override
		public List<? extends NarratableEntry> narratables() {
			return List.of(volume, mute);
		}
	}

	private static Component playerMuteLabel(boolean muted) {
		return Component.translatable(muted ? "yakvc.menu.unmute" : "yakvc.menu.mute");
	}

	/** 0 to 200 %: the slider's 0..1 is half the gain. */
	private static final class VolumeSlider extends AbstractSliderButton {
		private final Consumer<Float> onChange;

		VolumeSlider(float volume, Consumer<Float> onChange) {
			super(0, 0, 100, 20, Component.empty(), volume / 2.0);
			this.onChange = onChange;
			updateMessage();
		}

		@Override
		protected void updateMessage() {
			setMessage(Component.translatable("yakvc.menu.volume", Math.round(value * 200)));
		}

		@Override
		protected void applyValue() {
			onChange.accept((float) (Math.round(value * 200) / 100.0));
		}
	}
}
