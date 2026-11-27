package io.github.desocketed.yakvc;

import com.mojang.brigadier.arguments.DoubleArgumentType;
import com.mojang.brigadier.builder.LiteralArgumentBuilder;
import com.mojang.brigadier.builder.RequiredArgumentBuilder;
import io.github.desocketed.yakvc.config.Settings;
import io.github.desocketed.yakvc.heads.HeadDistort;
import io.github.desocketed.yakvc.heads.HeadStyle;
import io.github.desocketed.yakvc.ui.VoiceMenuScreen;
import io.github.desocketed.yakvc.ui.VoiceSettingsScreen;
import net.fabricmc.fabric.api.client.command.v2.ClientCommandRegistrationCallback;
import net.fabricmc.fabric.api.client.command.v2.FabricClientCommandSource;
import net.minecraft.client.Minecraft;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;

/**
 * The client command {@code /yakvc}: alone or with {@code settings} it opens the voice settings, {@code menu} the
 * voice menu, and {@code heads <style> [strength]} switches how voices move heads, and how far, and
 * {@code distort <name> [strength]} how they deform them, saved like a change in the settings, so they can be
 * compared without the menus.
 */
final class YakVcCommand {
	private YakVcCommand() {}

	/** What the command does, apart from its syntax, so the tree can be tested without a game. */
	interface Actions<S> {
		int settings();

		int menu();

		int heads(S source, HeadStyle style, @Nullable Double strength);

		int distort(S source, HeadDistort distort, @Nullable Double strength);
	}

	static void register(GameStateFeeder feeder) {
		Actions<FabricClientCommandSource> actions = new Actions<>() {
			@Override
			public int settings() {
				feeder.openSoon(() -> new VoiceSettingsScreen(null, Minecraft.getInstance().options, feeder));
				return 1;
			}

			@Override
			public int menu() {
				feeder.openSoon(() -> new VoiceMenuScreen(null, feeder));
				return 1;
			}

			@Override
			public int heads(FabricClientCommandSource source, HeadStyle style, @Nullable Double strength) {
				Settings changed = feeder.settings().withSpeakingHeadsStyle(style);
				if (strength != null) changed = changed.withSpeakingHeadsStrength(strength);
				if (!VoiceSettingsScreen.applyAndSave(feeder, changed)) return 0;
				source.sendFeedback(Component.translatable("yakvc.command.heads",
						Component.translatable("yakvc.settings.speaking_heads_style." + style.key()),
						changed.headsStrength()));
				return 1;
			}

			@Override
			public int distort(FabricClientCommandSource source, HeadDistort distort, @Nullable Double strength) {
				Settings changed = feeder.settings().withSpeakingHeadsDistort(distort);
				if (strength != null) changed = changed.withSpeakingHeadsDistortStrength(strength);
				if (!VoiceSettingsScreen.applyAndSave(feeder, changed)) return 0;
				source.sendFeedback(Component.translatable("yakvc.command.distort",
						Component.translatable("yakvc.settings.speaking_heads_distort." + distort.key()),
						changed.distortStrength()));
				return 1;
			}
		};
		ClientCommandRegistrationCallback.EVENT.register((dispatcher, context) -> dispatcher.register(tree(actions)));
	}

	static <S> LiteralArgumentBuilder<S> tree(Actions<S> actions) {
		LiteralArgumentBuilder<S> heads = LiteralArgumentBuilder.literal("heads");
		for (HeadStyle style : HeadStyle.values()) {
			heads.then(LiteralArgumentBuilder.<S>literal(style.key())
					.executes(c -> actions.heads(c.getSource(), style, null))
					.then(RequiredArgumentBuilder.<S, Double>argument("strength", DoubleArgumentType.doubleArg(0, Settings.MAX_HEADS_STRENGTH))
							.executes(c -> actions.heads(c.getSource(), style, DoubleArgumentType.getDouble(c, "strength")))));
		}
		LiteralArgumentBuilder<S> distort = LiteralArgumentBuilder.literal("distort");
		for (HeadDistort kind : HeadDistort.values()) {
			distort.then(LiteralArgumentBuilder.<S>literal(kind.key())
					.executes(c -> actions.distort(c.getSource(), kind, null))
					.then(RequiredArgumentBuilder.<S, Double>argument("strength", DoubleArgumentType.doubleArg(0, Settings.MAX_DISTORT_STRENGTH))
							.executes(c -> actions.distort(c.getSource(), kind, DoubleArgumentType.getDouble(c, "strength")))));
		}
		return LiteralArgumentBuilder.<S>literal("yakvc")
				.executes(c -> actions.settings())
				.then(LiteralArgumentBuilder.<S>literal("settings").executes(c -> actions.settings()))
				.then(LiteralArgumentBuilder.<S>literal("menu").executes(c -> actions.menu()))
				.then(heads)
				.then(distort);
	}
}
