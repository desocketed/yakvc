package io.github.desocketed.yakvc.ui;

import com.mojang.blaze3d.platform.InputConstants;
import com.mojang.serialization.Codec;
import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.UpdateCheck;
import io.github.desocketed.yakvc.YakVcClient;
import io.github.desocketed.yakvc.config.Settings;
import io.github.desocketed.yakvc.config.Settings.Acoustics;
import io.github.desocketed.yakvc.config.SpatialAudio;
import io.github.desocketed.yakvc.natives.EngineEvent;
import java.io.IOException;
import java.net.URI;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.function.Consumer;
import java.util.function.ToDoubleFunction;
import net.minecraft.client.Minecraft;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.AbstractWidget;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.EditBox;
import net.minecraft.client.gui.components.MultiLineTextWidget;
import net.minecraft.client.gui.components.Tooltip;
import net.minecraft.client.gui.screens.ConfirmLinkScreen;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.client.gui.screens.AlertScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.options.OptionsSubScreen;
import net.minecraft.network.chat.CommonComponents;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;
/**
 * The settings in {@code settings.json} that players are expected to change. Like the game's own options, a change
 * applies at once: the changed settings are checked and applied by the engine with {@code yakvc_update_config}, and
 * only then saved. A change the engine rejects puts the controls back to what is in effect.
 */
public final class VoiceSettingsScreen extends OptionsSubScreen {
	private static final String PUSH_TO_TALK = "push_to_talk";
	/** The ids of the engine's built-in test inputs ({@code TEST_INPUTS} in yakvc-audio). */
	private static final String TEST_INPUT_PREFIX = "yakvc:test-";
	/** The test inputs the Developer section offers, after "Off": all of {@code TEST_INPUTS}. */
	private static final List<String> TEST_SOUNDS = List.of("clicks", "noise", "sweep", "beeps", "tone");
	private static final String NO_TEST_SOUND = "";
	private static final String VOICE = "voice";
	/** Stands for an unset device: the system default microphone, or speakers that follow the game. */
	private static final String DEFAULT_DEVICE = "";
	/**
	 * The slider's top. The engine allows up to 1000%, but past 400% a quiet microphone mostly gets louder hiss; a
	 * hand-edited higher gain shows as 400% and is left alone until the slider moves.
	 */
	private static final int MAX_INPUT_GAIN_PERCENT = 400;
	/**
	 * The reverb and echo sliders' top. The engine allows up to 400%, as Sound Tuning offers, but twice the default is
	 * all a player needs; a higher value set there shows as 200% and is left alone until the slider moves.
	 */
	private static final int MAX_ACOUSTICS_PERCENT = 200;
	/** A server code is about 170 characters; room for a longer relay hostname. */
	private static final int MAX_SERVER_CODE_CHARS = 400;

	private final GameStateFeeder feeder;
	private final Settings initial;
	private final ApplyThrottle throttle = new ApplyThrottle();
	private final OptionInstance<String> activation;
	/** The voice activation threshold, in whole dBFS; only listed while activation is voice. */
	private final OptionInstance<Integer> vadThresholdDb;
	private final OptionInstance<Integer> range;
	private final OptionInstance<Integer> bitrateKbps;
	/** The microphone's gain, in percent. */
	private final OptionInstance<Integer> inputGainPercent;
	private final OptionInstance<Boolean> noiseSuppression;
	private final OptionInstance<SpatialAudio> spatialAudio;
	/** The room reverb's and the far echoes' loudness, in percent. Shown only with Spatial Audio on Full. */
	private final OptionInstance<Integer> reverbPercent;
	private final OptionInstance<Integer> echoPercent;
	/** Whether the reverb and echo sliders are in the list, so a Spatial Audio change can rebuild it. */
	private boolean acousticsShown;
	private final Dropdown inputDevice;
	private final Dropdown outputDevice;
	/** The device names, once listed: the listing runs on a worker thread while the screen shows "Loading…". */
	private GameStateFeeder.@Nullable Devices devices;
	/** Plays a test input instead of the microphone, for developers. */
	private final Dropdown testSound;
	/** The real microphone picked last, which turning the test sound off goes back to. */
	private String realInput;
	private final OptionInstance<Boolean> relayOnly;
	private final OptionInstance<Boolean> respectChatRestrictions;
	private final OptionInstance<Boolean> muteBlockedPlayers;
	private final OptionInstance<Boolean> verifiedOnly;
	private final OptionInstance<String> invites;
	private final OptionInstance<Boolean> friendsOnly;
	private final OptionInstance<Boolean> checkForUpdates;
	private final OptionInstance<Boolean> debugOverlay;
	/** The account line, which follows the rendezvous state while the screen is open. */
	private @Nullable MultiLineTextWidget accountText;
	private Component shownAccountState = Component.empty();
	/** The Voice Server code, read when the screen closes. */
	private @Nullable EditBox server;

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
		this.initial = feeder.settings();
		// Rebuilt so the threshold slider appears or goes with the choice.
		activation = activationOption(initial, value -> {
			throttle.changed();
			rebuildWidgets();
		});
		vadThresholdDb = new OptionInstance<>("yakvc.settings.vad_threshold_db",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.vad_threshold_db.tooltip")),
				(caption, value) -> Options.genericValueLabel(caption, Component.translatable("yakvc.settings.vad_threshold_db.decibels", value)),
				new OptionInstance.IntRange(-100, 0), vadThresholdDb(initial), value -> throttle.changed());
		invites = invitesOption(initial, value -> throttle.changed());
		int maxRange = options.renderDistance().get() * 16;
		range = new OptionInstance<>("yakvc.settings.range",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.range.tooltip")),
				(caption, value) -> Options.genericValueLabel(caption, Component.translatable("yakvc.settings.range.blocks", value)),
				new OptionInstance.IntRange(1, maxRange), (int) Math.clamp(Math.round(shownRange(initial)), 1, maxRange),
				value -> throttle.changed());
		bitrateKbps = new OptionInstance<>("yakvc.settings.bitrate",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.bitrate.tooltip")),
				(caption, value) -> Options.genericValueLabel(caption, Component.translatable("yakvc.settings.bitrate.kbps", value)),
				new OptionInstance.IntRange(16, 64), Math.clamp(initial.audio().bitrate() / 1000, 16, 64),
				value -> throttle.changed());
		noiseSuppression = OptionInstance.createBoolean("yakvc.settings.noise_suppression",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.noise_suppression.tooltip")),
				initial.audio().noiseSuppression(), value -> throttle.changed());
		spatialAudio = new OptionInstance<>("yakvc.settings.spatial_audio",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.spatial_audio.tooltip")),
				(caption, value) -> Component.translatable("yakvc.settings.spatial_audio." + value.key()),
				new OptionInstance.Enum<>(List.of(SpatialAudio.values()), Codec.STRING.xmap(SpatialAudio::of, SpatialAudio::key)),
				initial.spatialAudio(), value -> throttle.changed());
		reverbPercent = acousticsOption("reverb_level", initial, Acoustics::reverbLevel);
		echoPercent = acousticsOption("echo_level", initial, Acoustics::echoLevel);
		inputGainPercent = new OptionInstance<>("yakvc.settings.input_gain",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.input_gain.tooltip")),
				(caption, value) -> Options.genericValueLabel(caption, Component.translatable("yakvc.tuning.percent", value)),
				new OptionInstance.IntRange(0, MAX_INPUT_GAIN_PERCENT), inputGainPercent(initial), value -> throttle.changed());
		inputDevice = device("yakvc.settings.input_device", true, initial.audio().inputDevice(),
				"yakvc.settings.input_device.default");
		outputDevice = device("yakvc.settings.output_device", false, initial.audio().outputDevice(),
				"yakvc.settings.output_device.default");
		String input = inputDevice.get();
		realInput = input.startsWith(TEST_INPUT_PREFIX) ? DEFAULT_DEVICE : input;
		List<String> testSounds = new ArrayList<>(List.of(NO_TEST_SOUND));
		TEST_SOUNDS.forEach(sound -> testSounds.add(TEST_INPUT_PREFIX + sound));
		testSound = new Dropdown(Component.translatable("yakvc.settings.test_sound"), testSounds,
				testSounds.contains(input) ? input : NO_TEST_SOUND,
				value -> Component.translatable(value.isEmpty() ? "yakvc.settings.test_sound.off"
						: "yakvc.settings.test_sound." + value.substring(TEST_INPUT_PREFIX.length())));
		testSound.onPick(sound -> {
			useInput(sound.isEmpty() ? realInput : sound);
			throttle.changed();
		});
		inputDevice.onPick(mic -> {
			realInput = mic;
			testSound.set(NO_TEST_SOUND);
			throttle.changed();
		});
		outputDevice.onPick(speakers -> throttle.changed());
		// The engine builds its endpoint once, so this one waits for a restart, and its label says so until then.
		relayOnly = OptionInstance.createBoolean("yakvc.settings.relay_only",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.relay_only.tooltip")),
				(caption, value) -> value == feeder.startedRelayOnly() ? CommonComponents.optionStatus(value)
						: Component.translatable(value ? "yakvc.settings.relay_only.on_after_restart"
								: "yakvc.settings.relay_only.off_after_restart"),
				initial.relayOnly(), value -> throttle.changed());
		respectChatRestrictions = OptionInstance.createBoolean("yakvc.settings.respect_chat_restrictions",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.respect_chat_restrictions.tooltip")),
				initial.respectChatRestrictions(), value -> throttle.changed());
		muteBlockedPlayers = OptionInstance.createBoolean("yakvc.settings.mute_blocked_players",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.mute_blocked_players.tooltip")),
				initial.muteBlockedPlayers(), value -> throttle.changed());
		friendsOnly = OptionInstance.createBoolean("yakvc.settings.friends_only",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.friends_only.tooltip")),
				initial.friendsOnly(), value -> throttle.changed());
		verifiedOnly = OptionInstance.createBoolean("yakvc.settings.verified_only",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.verified_only.tooltip")),
				initial.verifiedOnly(), value -> throttle.changed());
		checkForUpdates = OptionInstance.createBoolean("yakvc.settings.check_for_updates",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.check_for_updates.tooltip")),
				initial.checkForUpdates(), value -> throttle.changed());
		// Unlike the settings above, it isn't saved.
		debugOverlay = OptionInstance.createBoolean("yakvc.settings.debug_overlay",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.debug_overlay.tooltip")),
				feeder.debugOverlay(), feeder::setDebugOverlay);
		// Last, after both pickers exist: a stopped engine answers at once.
		feeder.listDevices().thenAccept(this::fillDevices);
	}

	/** One of the {@code acoustics} levels (a fraction, 1 being unchanged) as a percent slider. */
	private OptionInstance<Integer> acousticsOption(String key, Settings settings, ToDoubleFunction<Acoustics> read) {
		return new OptionInstance<>("yakvc.settings." + key,
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings." + key + ".tooltip")),
				(caption, value) -> Options.genericValueLabel(caption, Component.translatable("yakvc.tuning.percent", value)),
				new OptionInstance.IntRange(0, MAX_ACOUSTICS_PERCENT), acousticsPercent(settings, read),
				value -> throttle.changed());
	}

	/** The Reverb ({@code reverb_level}) or Echo ({@code echo_level}) slider. Public for the client gametest. */
	public OptionInstance<Integer> acoustic(String key) {
		return key.equals("reverb_level") ? reverbPercent : echoPercent;
	}

	/** A saved acoustics level as its slider shows it, so an untouched slider never rewrites it. */
	private static int acousticsPercent(Settings settings, ToDoubleFunction<Acoustics> read) {
		return (int) Math.clamp(Math.round(read.applyAsDouble(settings.acoustics()) * 100), 0, MAX_ACOUSTICS_PERCENT);
	}

	/** Push to talk or voice activation. Shared with the voice menu's setup card. */
	static OptionInstance<String> activationOption(Settings settings, Consumer<String> onChange) {
		return new OptionInstance<>("yakvc.settings.activation", OptionInstance.noTooltip(),
				// Cycle buttons put the caption in front themselves; sliders (below) show only this text.
				(caption, value) -> Component.translatable("yakvc.settings.activation." + value),
				new OptionInstance.Enum<>(List.of(PUSH_TO_TALK, VOICE), Codec.STRING),
				settings.audio().voiceActivation() ? VOICE : PUSH_TO_TALK, onChange::accept);
	}

	/** Who may invite us to a group; friends are marked in the voice menu. Shared with the menu's setup card. */
	static OptionInstance<String> invitesOption(Settings settings, Consumer<String> onChange) {
		List<String> values = List.of("everyone", "friends", "nobody");
		return new OptionInstance<>("yakvc.settings.invites",
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.settings.invites.tooltip")),
				(caption, value) -> Component.translatable("yakvc.settings.invites." + value),
				new OptionInstance.Enum<>(values, Codec.STRING),
				values.contains(settings.invites()) ? settings.invites() : "everyone", onChange::accept);
	}

	/** {@code settings} with the activation and invites options' values. */
	static Settings withActivationAndInvites(Settings settings, OptionInstance<String> activation,
			OptionInstance<String> invites) {
		return settings.withActivation(activation.get()).withInvites(invites.get());
	}

	/**
	 * A device picker, showing the configured device, which gets the listed ones once they are known. Disabled until
	 * then: there is nothing to pick yet.
	 */
	private Dropdown device(String caption, boolean inputs, @Nullable String current, String defaultLabel) {
		Dropdown picker = new Dropdown(Component.translatable(caption), values(Map.of(), current),
				current == null ? DEFAULT_DEVICE : current, value -> {
					if (devices == null) return Component.translatable("yakvc.settings.devices_loading");
					if (value.isEmpty()) return Component.translatable(defaultLabel);
					Map<String, String> names = inputs ? devices.inputs() : devices.outputs();
					return Component.literal(names.getOrDefault(value, value));
				});
		picker.active = false;
		return picker;
	}

	/** The listed devices' ids, plus the configured one if it is unplugged right now. */
	private static List<String> values(Map<String, String> devices, @Nullable String current) {
		List<String> values = new ArrayList<>();
		values.add(DEFAULT_DEVICE);
		values.addAll(devices.keySet());
		if (current != null && !values.contains(current)) values.add(current);
		return values;
	}

	/**
	 * The microphones to offer, and {@code selected}. The engine's built-in test inputs are picked under Test Sound
	 * instead, so the list shows one only while it is in use.
	 */
	private List<String> inputValues(String selected) {
		Map<String, String> real = new LinkedHashMap<>(devices == null ? Map.of() : devices.inputs());
		real.keySet().removeIf(id -> id.startsWith(TEST_INPUT_PREFIX));
		return values(real, selected);
	}

	/** Switches the microphone picker to {@code id}, a real microphone or a test input. */
	private void useInput(String id) {
		inputDevice.setValues(inputValues(id));
		inputDevice.set(id);
	}

	/** On the client thread, possibly after the screen closed, which is harmless. */
	private void fillDevices(GameStateFeeder.Devices listed) {
		devices = listed;
		inputDevice.setValues(inputValues(inputDevice.get()));
		outputDevice.setValues(values(listed.outputs(), initial.audio().outputDevice()));
		inputDevice.active = true;
		outputDevice.active = true;
	}

	/** Whether the device pickers have their lists. For the client gametest. */
	public boolean devicesListed() {
		return devices != null;
	}

	@Override
	protected void init() {
		// init runs again on every rebuild, and the layout would keep the old widgets.
		layout.removeChildren();
		super.init();
	}

	@Override
	protected void addOptions() {
		list.addSmall(activation, range);
		if (activation.get().equals(VOICE)) list.addBig(vadThresholdDb);
		list.addSmall(bitrateKbps, noiseSuppression);
		list.addBig(inputDevice);
		list.addBig(inputGainPercent);
		list.addBig(new MicLevelMeter(feeder::micLevelDb, () -> vadThresholdDb.get()));
		list.addBig(outputDevice);
		list.addSmall(spatialAudio);
		// Only Full has reverb and echo: the other levels send the engine open air and no echoes.
		acousticsShown = spatialAudio.get() == SpatialAudio.FULL;
		if (acousticsShown) list.addSmall(reverbPercent, echoPercent);
		list.addHeader(Component.translatable("yakvc.settings.privacy"));
		list.addBig(new MultiLineTextWidget(Component.translatable("yakvc.settings.ip_note"), font)
				.setMaxWidth(310).setCentered(true));
		list.addSmall(relayOnly, respectChatRestrictions);
		list.addSmall(muteBlockedPlayers, invites);
		list.addSmall(friendsOnly);
		list.addSmall(checkForUpdates.createButton(options), releasesButton());
		list.addHeader(Component.translatable("yakvc.settings.account"));
		shownAccountState = accountState();
		accountText = new MultiLineTextWidget(shownAccountState, font).setMaxWidth(310).setCentered(true);
		list.addBig(accountText);
		list.addSmall(verifiedOnly);
		addVoiceServer();
		addDeveloperSection();
	}

	/**
	 * The Voice Server code, applied when the screen closes rather than as it is typed: a half-typed code is no
	 * server, and a change only takes effect after a restart anyway (the engine builds its endpoint once).
	 */
	private void addVoiceServer() {
		EditBox box = new EditBox(font, 0, 0, 150, 20, Component.translatable("yakvc.settings.server"));
		box.setMaxLength(MAX_SERVER_CODE_CHARS);
		box.setHint(Component.translatable("yakvc.settings.server.default"));
		box.setValue(initial.server() == null ? "" : initial.server());
		box.setTooltip(Tooltip.create(Component.translatable("yakvc.settings.server.tooltip")));
		server = box;
		list.addSmall(box, Button.builder(Component.translatable("yakvc.settings.server.reset"),
				button -> box.setValue("")).build());
		list.addBig(new MultiLineTextWidget(Component.translatable("yakvc.settings.server.note"), font)
				.setMaxWidth(310).setCentered(true));
	}

	/** The typed server code, or null for the default (preset or built-in) server. */
	private @Nullable String serverValue() {
		String typed = server == null ? "" : server.getValue().trim();
		return typed.isEmpty() ? null : typed;
	}

	private boolean serverChanged() {
		return !Objects.equals(serverValue(), feeder.settings().server());
	}

	/**
	 * Opens the releases page (after the game's usual link warning), naming the running version, or the newer one if
	 * the update check found it.
	 */
	private Button releasesButton() {
		String running = YakVcClient.version();
		UpdateCheck.Release newer = UpdateNotice.newer();
		Component label = newer == null ? Component.translatable("yakvc.settings.releases", running)
				: Component.translatable("yakvc.settings.releases.update", newer.version());
		URI page = newer == null ? UpdateCheck.RELEASES_PAGE : newer.page();
		return Button.builder(label, button -> ConfirmLinkScreen.confirmLinkNow(this, page))
				.tooltip(Tooltip.create(Component.translatable("yakvc.settings.releases.tooltip", running))).build();
	}

	/**
	 * Debug tools that players don't need, kept in one section so it can be hidden as a whole. The test sound is
	 * applied and saved with the other settings, since it is the microphone setting underneath.
	 */
	private void addDeveloperSection() {
		list.addHeader(Component.translatable("yakvc.settings.developer"));
		list.addSmall(debugOverlay.createButton(options), Button.builder(Component.translatable("yakvc.tuning.open"),
				button -> minecraft.gui.setScreen(new SoundTuningScreen(this, options, feeder))).build());
		list.addBig(testSound);
	}

	@Override
	public void tick() {
		super.tick();
		if (throttle.tick() && !apply(false)) {
			// Reopened from what is in effect, so no control shows a value the engine refused.
			minecraft.gui.setScreen(new VoiceSettingsScreen(lastScreen, options, feeder));
			return;
		}
		if (acousticsShown != (spatialAudio.get() == SpatialAudio.FULL)) showValues();
		Component now = accountState();
		if (accountText != null && !now.equals(shownAccountState)) {
			shownAccountState = now;
			accountText.setMessage(now);
		}
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
		testSound.extractList(graphics, mouseX, mouseY);
		outputDevice.extractList(graphics, mouseX, mouseY);
	}

	@Override
	public boolean mouseClicked(MouseButtonEvent event, boolean doubleClick) {
		if (event.button() == InputConstants.MOUSE_BUTTON_RIGHT && resetSlider(event)) return true;
		return inputDevice.listClicked(event.x(), event.y(), height)
				|| outputDevice.listClicked(event.x(), event.y(), height)
				|| testSound.listClicked(event.x(), event.y(), height)
				|| super.mouseClicked(event, doubleClick);
	}

	@Override
	public boolean mouseScrolled(double x, double y, double scrollX, double scrollY) {
		return inputDevice.listScrolled(scrollY) || outputDevice.listScrolled(scrollY)
				|| testSound.listScrolled(scrollY)
				|| super.mouseScrolled(x, y, scrollX, scrollY);
	}

	@Override
	public boolean keyPressed(KeyEvent event) {
		// Escape closes an open list rather than the screen.
		if (event.isEscape() && (inputDevice.isOpen() || outputDevice.isOpen() || testSound.isOpen())) {
			inputDevice.close();
			outputDevice.close();
			testSound.close();
			return true;
		}
		return super.keyPressed(event);
	}

	/** A right click on the threshold, reverb or echo slider puts it back to its default; true if one was hit. */
	private boolean resetSlider(MouseButtonEvent event) {
		for (OptionInstance<Integer> option : List.of(vadThresholdDb, reverbPercent, echoPercent)) {
			AbstractWidget widget = list.findOption(option);
			if (widget == null || !widget.isMouseOver(event.x(), event.y())) continue;
			option.set(defaultValue(option));
			throttle.changed();
			showValues();
			return true;
		}
		return false;
	}

	private int defaultValue(OptionInstance<Integer> option) {
		if (option == vadThresholdDb) return vadThresholdDb(Settings.DEFAULT);
		return acousticsPercent(Settings.DEFAULT, option == reverbPercent ? Acoustics::reverbLevel : Acoustics::echoLevel);
	}

	/** Rebuilds the controls, which only read their option when made, keeping the list where it was scrolled to. */
	private void showValues() {
		double scroll = list.scrollAmount();
		rebuildWidgets();
		list.setScrollAmount(scroll);
	}

	/** Called whenever the screen goes away: a change still waiting on the throttle is applied now. */
	@Override
	public void removed() {
		super.removed();
		if (throttle.takePending() || serverChanged()) apply(true);
	}

	private double shownRange(Settings settings) {
		return Settings.effectiveVoiceRange(settings.voiceRange(), options.renderDistance().get());
	}

	/**
	 * Applies and saves what differs from the settings in effect; false if the engine rejected it. The sliders show
	 * rounded values, so each is only written when it moved, or a hand-edited value would be rewritten on every close.
	 */
	private boolean apply(boolean withServer) {
		Settings current = feeder.settings();
		Settings changed = withActivationAndInvites(current, activation, invites)
				.withRelayOnly(relayOnly.get())
				.withRespectChatRestrictions(respectChatRestrictions.get())
				.withMuteBlockedPlayers(muteBlockedPlayers.get())
				.withVerifiedOnly(verifiedOnly.get())
				.withFriendsOnly(friendsOnly.get())
				.withCheckForUpdates(checkForUpdates.get())
				.withSpatialAudio(spatialAudio.get())
				.withNoiseSuppression(noiseSuppression.get())
				.withInputDevice(deviceValue(inputDevice.get()))
				.withOutputDevice(deviceValue(outputDevice.get()));
		if (withServer) changed = changed.withServer(serverValue());
		// Against the capped range, so a saved range past the render distance stays saved unless moved.
		if (range.get() != Math.round(shownRange(current))) changed = changed.withVoiceRange(range.get());
		if (bitrateKbps.get() != current.audio().bitrate() / 1000) changed = changed.withBitrate(bitrateKbps.get() * 1000);
		if (inputGainPercent.get() != inputGainPercent(current)) {
			changed = changed.withInputGain(inputGainPercent.get() / 100.0);
		}
		if (vadThresholdDb.get() != vadThresholdDb(current)) changed = changed.withVadThresholdDb(vadThresholdDb.get());
		if (reverbPercent.get() != acousticsPercent(current, Acoustics::reverbLevel)) {
			changed = changed.withAcoustic("reverb_level", reverbPercent.get() / 100.0);
		}
		if (echoPercent.get() != acousticsPercent(current, Acoustics::echoLevel)) {
			changed = changed.withAcoustic("echo_level", echoPercent.get() / 100.0);
		}
		return changed.equals(current) || applyAndSave(feeder, changed);
	}

	/**
	 * Has the engine check and apply {@code changed}, and saves it if it took it; says so if anything failed. False if
	 * the engine rejected it.
	 */
	public static boolean applyAndSave(GameStateFeeder feeder, Settings changed) {
		String error = feeder.applySettings(changed);
		if (error != null) {
			VoiceToasts.show(Component.translatable("yakvc.toast.settings_rejected", error));
			return false;
		}
		try {
			changed.save(YakVcClient.configDir());
		} catch (IOException e) {
			YakVcClient.LOGGER.error("Could not save voice settings", e);
			VoiceToasts.show(Component.translatable("yakvc.toast.settings_not_saved"));
		}
		return true;
	}

	/** The saved microphone gain as the slider shows it, so an untouched slider never rewrites it. */
	private static int inputGainPercent(Settings settings) {
		return (int) Math.clamp(Math.round(settings.audio().inputGain() * 100), 0, MAX_INPUT_GAIN_PERCENT);
	}

	/** The saved threshold as the slider shows it, likewise. */
	private static int vadThresholdDb(Settings settings) {
		return (int) Math.clamp(Math.round(settings.audio().vadThresholdDb()), -100, 0);
	}

	private static @Nullable String deviceValue(String id) {
		return id.equals(DEFAULT_DEVICE) ? null : id;
	}
}
