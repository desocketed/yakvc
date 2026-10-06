package io.github.desocketed.yakvc.ui;

import com.mojang.serialization.Codec;
import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.YakVcClient;
import io.github.desocketed.yakvc.config.ClientConfig;
import io.github.desocketed.yakvc.natives.EngineEvent;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.Objects;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.gui.components.MultiLineTextWidget;
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
	/** Stands for an unset device name: the system default microphone, or speakers that follow the game. */
	private static final String DEFAULT_DEVICE = "";

	private final GameStateFeeder feeder;
	private final ClientConfig initial;
	private final OptionInstance<String> activation;
	private final OptionInstance<Integer> range;
	private final OptionInstance<Integer> bitrateKbps;
	private final OptionInstance<String> inputDevice;
	private final OptionInstance<String> outputDevice;
	private final OptionInstance<Boolean> relayOnly;
	private final OptionInstance<Boolean> respectChatRestrictions;
	private final OptionInstance<Boolean> muteBlockedPlayers;
	private final OptionInstance<Boolean> verifiedOnly;

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
		inputDevice = device("yakvc.settings.input_device", feeder.deviceNames(true), initial.inputDevice(),
				"yakvc.settings.input_device.default");
		outputDevice = device("yakvc.settings.output_device", feeder.deviceNames(false), initial.outputDevice(),
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
	}

	/** A device picker over the listed names, plus the configured one if it is unplugged right now. */
	private static OptionInstance<String> device(String caption, List<String> names, @Nullable String current,
			String defaultLabel) {
		List<String> values = new ArrayList<>();
		values.add(DEFAULT_DEVICE);
		values.addAll(names);
		if (current != null && !values.contains(current)) values.add(current);
		return new OptionInstance<>(caption, OptionInstance.noTooltip(),
				(label, value) -> value.isEmpty() ? Component.translatable(defaultLabel) : Component.literal(value),
				new OptionInstance.Enum<>(values, Codec.STRING), current == null ? DEFAULT_DEVICE : current,
				OptionInstance.NO_ACTION);
	}

	@Override
	protected void addOptions() {
		list.addSmall(activation, range);
		list.addSmall(bitrateKbps);
		list.addBig(inputDevice);
		list.addBig(outputDevice);
		list.addHeader(Component.translatable("yakvc.settings.privacy"));
		list.addBig(new MultiLineTextWidget(Component.translatable("yakvc.settings.ip_note"), font)
				.setMaxWidth(310).setCentered(true));
		list.addSmall(relayOnly, respectChatRestrictions);
		list.addSmall(muteBlockedPlayers);
		list.addHeader(Component.translatable("yakvc.settings.account"));
		list.addBig(new MultiLineTextWidget(accountState(), font).setMaxWidth(310).setCentered(true));
		list.addSmall(verifiedOnly);
	}

	/** Whether the rendezvous verified our Mojang account. */
	private Component accountState() {
		if (feeder.rendezvous() != EngineEvent.RendezvousState.REGISTERED) {
			return Component.translatable("yakvc.settings.account.not_connected");
		}
		return Component.translatable(feeder.verified() ? "yakvc.settings.account.verified"
				: "yakvc.settings.account.unverified");
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

	private static @Nullable String deviceValue(String name) {
		return name.equals(DEFAULT_DEVICE) ? null : ClientConfig.quote(name);
	}
}
