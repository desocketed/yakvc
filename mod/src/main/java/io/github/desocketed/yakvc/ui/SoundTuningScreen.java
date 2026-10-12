package io.github.desocketed.yakvc.ui;

import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.config.ClientConfig;
import io.github.desocketed.yakvc.config.SoundTuning;
import java.util.ArrayList;
import java.util.List;
import java.util.function.DoubleFunction;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.options.OptionsSubScreen;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;

/**
 * Advanced sound tuning, for adjusting by ear how voices carry through the world: distance, walls, rooms and echo.
 * Every change applies at once, so you can listen while dragging, and is saved to {@code [acoustics]} in
 * {@code client.toml} when the screen closes. Reached from the settings' Troubleshooting section.
 */
public final class SoundTuningScreen extends OptionsSubScreen {
	private final GameStateFeeder feeder;
	private final ClientConfig initial;
	private final List<Slider> sliders = new ArrayList<>();

	/** One {@code [acoustics]} value as a slider whose whole steps, times {@code step}, give the value. */
	private record Slider(String key, double step, double fallback, OptionInstance<Integer> option) {
		double value() {
			return option.get() * step;
		}

		void reset() {
			option.set((int) Math.round(fallback / step));
		}
	}

	public SoundTuningScreen(@Nullable Screen lastScreen, Options options, GameStateFeeder feeder) {
		super(lastScreen, options, Component.translatable("yakvc.tuning.title"));
		this.feeder = feeder;
		this.initial = feeder.config();
		SoundTuning mod = SoundTuning.DEFAULT;
		// The engine's keys fall back to its own defaults (AcousticsConfig); the mod's to SoundTuning's.
		slider("full_volume_distance", 0, 32, 1, 4, SoundTuningScreen::blocks);
		slider("distance_rolloff", 10, 300, 0.01, 0.6, value -> label("times", "%.2f", value));
		slider("occlusion_attenuation", 0, 100, 0.01, 0.75, SoundTuningScreen::percent);
		slider("muffled_cutoff_hz", 2, 80, 50, 600, value -> label("hertz", "%.0f", value));
		slider("path_bend", 0, 100, 0.01, mod.pathBend(), SoundTuningScreen::percent);
		slider("path_per_block", 0, 100, 0.001, mod.pathPerBlock(), SoundTuningScreen::percent);
		slider("reverb_level", 0, 400, 0.01, 0.85, SoundTuningScreen::percent);
		slider("reverb_damping", 0, 95, 0.01, 0.25, SoundTuningScreen::percent);
		slider("room_reach", 8, 128, 1, mod.roomReach(), SoundTuningScreen::blocks);
		slider("furnishing", 0, 100, 0.01, mod.furnishing(), SoundTuningScreen::percent);
		slider("max_decay", 5, 100, 0.1, mod.maxDecay(), value -> label("seconds", "%.1f", value));
		slider("echo_feedback", 0, 90, 0.01, 0.4, SoundTuningScreen::percent);
	}

	private void slider(String key, int min, int max, double step, double fallback, DoubleFunction<Component> label) {
		int steps = (int) Math.clamp(Math.round(initial.acoustics(key, fallback) / step), min, max);
		OptionInstance<Integer> option = new OptionInstance<>("yakvc.tuning." + key,
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.tuning." + key + ".tooltip")),
				(caption, value) -> Options.genericValueLabel(caption, label.apply(value * step)),
				new OptionInstance.IntRange(min, max), steps, value -> apply());
		sliders.add(new Slider(key, step, fallback, option));
	}

	@Override
	protected void addOptions() {
		list.addHeader(Component.translatable("yakvc.tuning.distance"));
		list.addSmall(option("full_volume_distance"), option("distance_rolloff"));
		list.addHeader(Component.translatable("yakvc.tuning.walls"));
		list.addSmall(option("occlusion_attenuation"), option("muffled_cutoff_hz"));
		list.addSmall(option("path_bend"), option("path_per_block"));
		list.addHeader(Component.translatable("yakvc.tuning.rooms"));
		list.addSmall(option("reverb_level"), option("reverb_damping"));
		list.addSmall(option("furnishing"), option("max_decay"));
		list.addSmall(option("room_reach"), option("echo_feedback"));
		list.addBig(Button.builder(Component.translatable("yakvc.tuning.reset"), button -> {
			sliders.forEach(Slider::reset);
			apply();
			rebuildWidgets();
		}).build());
	}

	/** The slider for an {@code [acoustics]} key. Public for the client gametest. */
	public OptionInstance<Integer> option(String key) {
		return sliders.stream().filter(slider -> slider.key().equals(key)).findFirst().orElseThrow().option();
	}

	/** The config with every slider's value, writing only the keys whose value the player changed. */
	private ClientConfig tuned() {
		ClientConfig config = initial;
		for (Slider slider : sliders) {
			if (Math.abs(slider.value() - initial.acoustics(slider.key(), slider.fallback())) > slider.step() / 2) {
				// Rounded, so the file reads 0.35 rather than 0.35000000000000003.
				double value = Math.round(slider.value() * 10_000) / 10_000.0;
				config = config.with("acoustics", slider.key(), String.valueOf(value));
			}
		}
		return config;
	}

	/** Hands the engine the tuned config at once, so a change is heard while dragging. Saved on close. */
	private void apply() {
		feeder.applyConfig(tuned());
	}

	@Override
	public void removed() {
		super.removed();
		ClientConfig tuned = tuned();
		if (!tuned.equals(initial)) VoiceSettingsScreen.applyAndSave(feeder, tuned);
	}

	/** Whole percents as such, and a decimal only where the step is finer, as for the detour cost. */
	private static Component percent(double fraction) {
		double percent = fraction * 100;
		return label("percent", Math.abs(percent - Math.round(percent)) < 0.05 ? "%.0f" : "%.1f", percent);
	}

	private static Component blocks(double value) {
		return Component.translatable("yakvc.settings.range.blocks", (int) value);
	}

	private static Component label(String unit, String format, double value) {
		return Component.translatable("yakvc.tuning." + unit, String.format(format, value));
	}
}
