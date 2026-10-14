package io.github.desocketed.yakvc.ui;

import com.mojang.blaze3d.platform.InputConstants;
import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.config.Settings;
import io.github.desocketed.yakvc.config.Settings.Acoustics;
import java.util.ArrayList;
import java.util.List;
import java.util.Objects;
import java.util.function.DoubleFunction;
import java.util.function.ToDoubleFunction;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.gui.components.AbstractWidget;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.Tooltip;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.options.OptionsSubScreen;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;

/**
 * Advanced sound tuning, for adjusting by ear how voices carry through the world: distance, walls, rooms and echo.
 * Every change applies at once, so you can listen while dragging, and is saved to {@code acoustics} in
 * {@code settings.json} when the screen closes. Reached from the settings' Developer section; the settings' own Reverb
 * and Echo sliders edit two of the same values.
 */
public final class SoundTuningScreen extends OptionsSubScreen {
	private final GameStateFeeder feeder;
	private final Settings initial;
	private final List<Slider> sliders = new ArrayList<>();

	/**
	 * One {@code acoustics} value as a slider whose whole steps, times {@code step}, give the value. {@code read} is
	 * the value's accessor, {@code key} its name in the settings.
	 */
	private record Slider(String key, ToDoubleFunction<Acoustics> read, double step, OptionInstance<Integer> option) {
		double value() {
			return option.get() * step;
		}

		void reset() {
			option.set((int) Math.round(read.applyAsDouble(Acoustics.DEFAULT) / step));
		}
	}

	public SoundTuningScreen(@Nullable Screen lastScreen, Options options, GameStateFeeder feeder) {
		super(lastScreen, options, Component.translatable("yakvc.tuning.title"));
		this.feeder = feeder;
		this.initial = feeder.settings();
		slider("full_volume_distance", Acoustics::fullVolumeDistance, 0, 32, 1, SoundTuningScreen::blocks);
		slider("distance_rolloff", Acoustics::distanceRolloff, 10, 400, 0.01, value -> label("times", "%.2f", value));
		slider("occlusion_attenuation", Acoustics::occlusionAttenuation, 0, 100, 0.01, SoundTuningScreen::percent);
		slider("muffled_cutoff_hz", Acoustics::muffledCutoffHz, 2, 80, 50, value -> label("hertz", "%.0f", value));
		slider("path_bend", Acoustics::pathBend, 0, 100, 0.01, SoundTuningScreen::percent);
		slider("path_per_block", Acoustics::pathPerBlock, 0, 100, 0.001, SoundTuningScreen::percent);
		slider("reverb_level", Acoustics::reverbLevel, 0, 400, 0.01, SoundTuningScreen::percent);
		slider("reverb_damping", Acoustics::reverbDamping, 0, 95, 0.01, SoundTuningScreen::percent);
		slider("room_reach", Acoustics::roomReach, 8, 128, 1, SoundTuningScreen::blocks);
		slider("furnishing", Acoustics::furnishing, 0, 100, 0.01, SoundTuningScreen::percent);
		slider("max_decay", Acoustics::maxDecay, 5, 100, 0.1, value -> label("seconds", "%.1f", value));
		slider("echo_feedback", Acoustics::echoFeedback, 0, 90, 0.01, SoundTuningScreen::percent);
		slider("echo_reach", Acoustics::echoReach, 32, 256, 1, SoundTuningScreen::blocks);
		slider("echo_level", Acoustics::echoLevel, 0, 400, 0.01, SoundTuningScreen::percent);
	}

	private void slider(String key, ToDoubleFunction<Acoustics> read, int min, int max, double step,
			DoubleFunction<Component> label) {
		int steps = (int) Math.clamp(Math.round(read.applyAsDouble(initial.acoustics()) / step), min, max);
		OptionInstance<Integer> option = new OptionInstance<>("yakvc.tuning." + key,
				OptionInstance.cachedConstantTooltip(Component.translatable("yakvc.tuning." + key + ".tooltip")),
				(caption, value) -> Options.genericValueLabel(caption, label.apply(value * step)),
				new OptionInstance.IntRange(min, max), steps, value -> apply());
		sliders.add(new Slider(key, read, step, option));
	}

	/** The slider widget for an {@code acoustics} key, for the client gametest to click. */
	public AbstractWidget widget(String key) {
		return Objects.requireNonNull(list.findOption(option(key)));
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
		list.addSmall(option("echo_reach"), option("echo_level"));
		list.addBig(Button.builder(Component.translatable("yakvc.tuning.reset"), button -> {
			sliders.forEach(this::reset);
			apply();
		}).tooltip(Tooltip.create(Component.translatable("yakvc.tuning.reset.tooltip"))).build());
	}

	/** A right click on a slider puts just that one back to its default. */
	@Override
	public boolean mouseClicked(MouseButtonEvent event, boolean doubleClick) {
		if (event.button() == InputConstants.MOUSE_BUTTON_RIGHT) {
			for (Slider slider : sliders) {
				AbstractWidget widget = list.findOption(slider.option());
				if (widget != null && widget.isMouseOver(event.x(), event.y())) {
					reset(slider);
					apply();
					return true;
				}
			}
		}
		return super.mouseClicked(event, doubleClick);
	}

	/**
	 * Puts a slider back to its default and shows it. The widget only reads its option when made, so it is told to
	 * read it again instead of being rebuilt: rebuilding the screen runs {@code init} again, which adds a second
	 * title, list and Done button to the layout, and the stacked lists drew their shading twice.
	 */
	private void reset(Slider slider) {
		slider.reset();
		resetOption(slider.option());
	}

	/** The slider for an {@code acoustics} key. Public for the client gametest. */
	public OptionInstance<Integer> option(String key) {
		return sliders.stream().filter(slider -> slider.key().equals(key)).findFirst().orElseThrow().option();
	}

	/** The settings with every slider's value, writing only the keys whose value the player changed. */
	private Settings tuned() {
		Settings settings = initial;
		for (Slider slider : sliders) {
			if (Math.abs(slider.value() - slider.read().applyAsDouble(initial.acoustics())) > slider.step() / 2) {
				// Rounded, so the file reads 0.35 rather than 0.35000000000000003.
				double value = Math.round(slider.value() * 10_000) / 10_000.0;
				settings = settings.withAcoustic(slider.key(), value);
			}
		}
		return settings;
	}

	/** Hands the engine the tuned settings at once, so a change is heard while dragging. Saved on close. */
	private void apply() {
		feeder.applySettings(tuned());
	}

	@Override
	public void removed() {
		super.removed();
		Settings tuned = tuned();
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
