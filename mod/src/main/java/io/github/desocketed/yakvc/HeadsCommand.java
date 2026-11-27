package io.github.desocketed.yakvc;

import static net.fabricmc.fabric.api.client.command.v2.ClientCommands.argument;
import static net.fabricmc.fabric.api.client.command.v2.ClientCommands.literal;

import com.mojang.brigadier.arguments.DoubleArgumentType;
import com.mojang.brigadier.builder.LiteralArgumentBuilder;
import com.mojang.brigadier.context.CommandContext;
import io.github.desocketed.yakvc.config.Settings;
import io.github.desocketed.yakvc.heads.HeadStyle;
import io.github.desocketed.yakvc.ui.VoiceSettingsScreen;
import net.fabricmc.fabric.api.client.command.v2.ClientCommandRegistrationCallback;
import net.fabricmc.fabric.api.client.command.v2.FabricClientCommandSource;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;

/**
 * {@code /yakvc heads <style> [strength]}: switches how voices move heads, and how far, without the menus, so styles
 * can be compared quickly; saved like a change in the settings.
 */
final class HeadsCommand {
	private HeadsCommand() {}

	static void register(GameStateFeeder feeder) {
		ClientCommandRegistrationCallback.EVENT.register((dispatcher, context) -> {
			LiteralArgumentBuilder<FabricClientCommandSource> heads = literal("heads");
			for (HeadStyle style : HeadStyle.values()) {
				heads.then(literal(style.key())
						.executes(c -> set(c, feeder, style, null))
						.then(argument("strength", DoubleArgumentType.doubleArg(0, Settings.MAX_HEADS_STRENGTH))
								.executes(c -> set(c, feeder, style, DoubleArgumentType.getDouble(c, "strength")))));
			}
			dispatcher.register(literal("yakvc").then(heads));
		});
	}

	private static int set(CommandContext<FabricClientCommandSource> context, GameStateFeeder feeder, HeadStyle style,
			@Nullable Double strength) {
		Settings changed = feeder.settings().withSpeakingHeadsStyle(style);
		if (strength != null) changed = changed.withSpeakingHeadsStrength(strength);
		if (!VoiceSettingsScreen.applyAndSave(feeder, changed)) return 0;
		context.getSource().sendFeedback(Component.translatable("yakvc.command.heads",
				Component.translatable("yakvc.settings.speaking_heads_style." + style.key()), changed.headsStrength()));
		return 1;
	}
}
