package io.github.desocketed.yakvc.ui;

import com.mojang.serialization.Codec;
import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.YakVcClient;
import io.github.desocketed.yakvc.config.ClientConfig;
import io.github.desocketed.yakvc.natives.EngineEvent;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import net.minecraft.client.Minecraft;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.MultiLineTextWidget;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.client.gui.screens.AlertScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.options.OptionsSubScreen;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;
/**
 * The settings in {@code client.toml} that players are expected to change. When the screen closes, changed values are
 * written into the file text, checked and applied by the engine with {@code yakvc_update_config}, and only then saved.
 */
public final class VoiceSettingsScreen extends OptionsSubScreen {
	private static final String PUSH_TO_TALK = "push_to_talk";
	private static final String VOICE = "voice";
	/** Stands for an unset device: the system default microphone, or speakers that follow the game. */
	private static final String DEFAULT_DEVICE = "";

	private final GameStateFeeder feeder;
	private final ClientConfig initial;
	private final OptionInstance<String> activation;
	private final OptionInstance<Integer> range;
	private final OptionInstance<Integer> bitrateKbps;
	private final Dropdown inputDevice;
	private final Dropdown outputDevice;
	private final OptionInstance<Boolean> relayOnly;
	private final OptionInstance<Boolean> respectChatRestrictions;
	private final OptionInstance<Boolean> muteBlockedPlayers;
	private final OptionInstance<Boolean> verifiedOnly;
	private final OptionInstance<Boolean> debugOverlay;

	/**
	 * The settings over {@code parent}, or an alert if the engine isn't running: the settings need it to check and
	 * apply changes.
	 */
	public static Screen open(Screen parent) {
		Minecraft minecraft = Minecraft.getInstance();
		GameStateFeeder feeder = YakVcClient.feeder();
		if (feeder == null) {
			return new AlertScreen(() -> minecraft.gui.setScreen(parent), Component.translatable("yakvc.settings.title"),
					Component.translatable("yakvc.settings.unavailable"));
		}
		return new VoiceSettingsScreen(parent, minecraft.options, feeder);
	}

	public VoiceSettingsScreen(@Nullable Screen lastScreen, Options options, GameStateFeeder feeder) {
		super(lastScreen, options, Component.translatable("yakvc.settings.title"));
		this.feeder = feeder;
		this.initial = feeder.config();
		activation = new OptionInstance<>("yakvc.settings.activation", OptionInstance.noTooltip(),
				// Cycle buttons put the caption in front themselves; sliders (below) show only this text.
				(caption, value) -> Component.translatable("yakvc.settings.activation." + value),
				new OptionInstance.Enum<>(List.of(PUSH_TO_TALK, VOICE), Codec.STRING),
				initial.voiceActivation() ? VOICE : PUSH_TO_TALK, OptionInstance.NO_ACTION);
		range = new OptionInstance<>("yakvc.settings.range", OptionInstance.noTooltip(),
				(caption, value) -> Options.genericValueLabel(caption, Component.translatable("yakvc.settings.range.blocks", value)),
				new OptionInstance.IntRange(1, 256), (int) Math.clamp(Math.round(initial.voiceRange()), 1, 256),
				OptionInstance.NO_ACTION);
		bitrateKbps = new OptionInstance<>("yakvc.settings.bitrate",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.bitrate.tooltip")),
				(caption, value) -> Options.genericValueLabel(caption, Component.translatable("yakvc.settings.bitrate.kbps", value)),
				new OptionInstance.IntRange(16, 64), Math.clamp(initial.bitrate() / 1000, 16, 64), OptionInstance.NO_ACTION);
		inputDevice = device("yakvc.settings.input_device", feeder.devices(true), initial.inputDevice(),
				"yakvc.settings.input_device.default");
		outputDevice = device("yakvc.settings.output_device", feeder.devices(false), initial.outputDevice(),
				"yakvc.settings.output_device.default");
		relayOnly = OptionInstance.createBoolean("yakvc.settings.relay_only",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.relay_only.tooltip")),
				initial.relayOnly());
		respectChatRestrictions = OptionInstance.createBoolean("yakvc.settings.respect_chat_restrictions",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.respect_chat_restrictions.tooltip")),
				initial.respectChatRestrictions());
		muteBlockedPlayers = OptionInstance.createBoolean("yakvc.settings.mute_blocked_players",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.mute_blocked_players.tooltip")),
				initial.muteBlockedPlayers());
		verifiedOnly = OptionInstance.createBoolean("yakvc.settings.verified_only",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.verified_only.tooltip")),
				initial.verifiedOnly());
		// Takes effect at once and isn't saved, unlike the settings above.
		debugOverlay = OptionInstance.createBoolean("yakvc.settings.debug_overlay",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.debug_overlay.tooltip")),
				feeder.debugOverlay(), feeder::setDebugOverlay);
	}

	/**
	 * A device picker over the listed devices' ids, shown by name, plus the configured one if it is unplugged right
	 * now. The engine also accepts a name there, as configs from before device ids hold.
	 */
	private static Dropdown device(String caption, Map<String, String> devices, @Nullable String current,
			String defaultLabel) {
		List<String> values = new ArrayList<>();
		values.add(DEFAULT_DEVICE);
		values.addAll(devices.keySet());
		if (current != null && !values.contains(current)) values.add(current);
		return new Dropdown(Component.translatable(caption), values, current == null ? DEFAULT_DEVICE : current,
				value -> value.isEmpty() ? Component.translatable(defaultLabel)
						: Component.literal(devices.getOrDefault(value, value)));
	}

	@Override
	protected void addOptions() {
		list.addSmall(activation, range);
		list.addSmall(bitrateKbps);
		list.addBig(inputDevice);
		list.addBig(new MicLevelMeter(feeder::micLevelDb, initial.vadThresholdDb()));
		list.addBig(outputDevice);
		list.addHeader(Component.translatable("yakvc.settings.privacy"));
		list.addBig(new MultiLineTextWidget(Component.translatable("yakvc.settings.ip_note"), font)
				.setMaxWidth(310).setCentered(true));
		list.addSmall(relayOnly, respectChatRestrictions);
		list.addSmall(muteBlockedPlayers);
		list.addHeader(Component.translatable("yakvc.settings.account"));
		list.addBig(new MultiLineTextWidget(accountState(), font).setMaxWidth(310).setCentered(true));
		list.addSmall(verifiedOnly);
		list.addHeader(Component.translatable("yakvc.settings.troubleshooting"));
		list.addSmall(debugOverlay);
	}

	/** Whether the rendezvous verified our Mojang account. */
	private Component accountState() {
		if (feeder.rendezvous() != EngineEvent.RendezvousState.REGISTERED) {
			return Component.translatable("yakvc.settings.account.not_connected");
		}
		return Component.translatable(feeder.verified() ? "yakvc.settings.account.verified"
				: "yakvc.settings.account.unverified");
	}

	// The open device list lies over the options list, so it is drawn last and gets input first.

	@Override
	public void extractRenderState(GuiGraphicsExtractor graphics, int mouseX, int mouseY, float a) {
		super.extractRenderState(graphics, mouseX, mouseY, a);
		inputDevice.extractList(graphics, mouseX, mouseY);
		outputDevice.extractList(graphics, mouseX, mouseY);
	}

	@Override
	public boolean mouseClicked(MouseButtonEvent event, boolean doubleClick) {
		return inputDevice.listClicked(event.x(), event.y(), height)
				|| outputDevice.listClicked(event.x(), event.y(), height)
				|| super.mouseClicked(event, doubleClick);
	}

	@Override
	public boolean mouseScrolled(double x, double y, double scrollX, double scrollY) {
		return inputDevice.listScrolled(scrollY) || outputDevice.listScrolled(scrollY)
				|| super.mouseScrolled(x, y, scrollX, scrollY);
	}

	@Override
	public boolean keyPressed(KeyEvent event) {
		// Escape closes an open list rather than the screen.
		if (event.isEscape() && (inputDevice.isOpen() || outputDevice.isOpen())) {
			inputDevice.close();
			outputDevice.close();
			return true;
		}
		return super.keyPressed(event);
	}

	/** Called whenever the screen goes away, after slider values have been applied. */
	@Override
	public void removed() {
		super.removed();
		// Only what the player changed is written, so the rest of the file keeps its exact text.
		ClientConfig changed = initial;
		if (range.get() != Math.round(initial.voiceRange())) {
			changed = changed.with("", "voice_range", String.valueOf((double) range.get()));
		}
		if (relayOnly.get() != initial.relayOnly()) {
			changed = changed.with("", "relay_only", String.valueOf(relayOnly.get()));
		}
		if (respectChatRestrictions.get() != initial.respectChatRestrictions()) {
			changed = changed.with("", "respect_chat_restrictions", String.valueOf(respectChatRestrictions.get()));
		}
		if (muteBlockedPlayers.get() != initial.muteBlockedPlayers()) {
			changed = changed.with("", "mute_blocked_players", String.valueOf(muteBlockedPlayers.get()));
		}
		if (verifiedOnly.get() != initial.verifiedOnly()) {
			changed = changed.with("", "verified_only", String.valueOf(verifiedOnly.get()));
		}
		if (activation.get().equals(VOICE) != initial.voiceActivation()) {
			changed = changed.with("audio", "activation", ClientConfig.quote(activation.get()));
		}
		if (bitrateKbps.get() * 1000 != initial.bitrate()) {
			changed = changed.with("audio", "bitrate", String.valueOf(bitrateKbps.get() * 1000));
		}
		if (!inputDevice.get().equals(Objects.requireNonNullElse(initial.inputDevice(), DEFAULT_DEVICE))) {
			changed = changed.with("audio", "input_device", deviceValue(inputDevice.get()));
		}
		if (!outputDevice.get().equals(Objects.requireNonNullElse(initial.outputDevice(), DEFAULT_DEVICE))) {
			changed = changed.with("audio", "output_device", deviceValue(outputDevice.get()));
		}
		if (changed.equals(initial)) return;
		String error = feeder.applyConfig(changed);
		if (error != null) {
			VoiceToasts.show(Component.translatable("yakvc.toast.settings_rejected", error));
			return;
		}
		try {
			changed.save(YakVcClient.configDir());
		} catch (IOException e) {
			YakVcClient.LOGGER.error("Could not save voice settings", e);
			VoiceToasts.show(Component.translatable("yakvc.toast.settings_not_saved"));
		}
	}

	private static @Nullable String deviceValue(String id) {
		return id.equals(DEFAULT_DEVICE) ? null : ClientConfig.quote(id);
	}
}
