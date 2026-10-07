package io.github.desocketed.yakvc;

import com.mojang.authlib.exceptions.AuthenticationException;
import io.github.desocketed.yakvc.config.ClientConfig;
import io.github.desocketed.yakvc.config.PlayerVolumes;
import io.github.desocketed.yakvc.input.VoiceKeys;
import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.natives.NativeLoader;
import io.github.desocketed.yakvc.ui.DebugOverlay;
import io.github.desocketed.yakvc.ui.TalkingIndicator;
import io.github.desocketed.yakvc.ui.VoiceHud;
import io.github.desocketed.yakvc.ui.VoiceToasts;
import java.lang.foreign.MemorySegment;
import java.nio.file.Path;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientLifecycleEvents;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.fabricmc.fabric.api.client.networking.v1.ClientConfigurationConnectionEvents;
import net.fabricmc.fabric.api.client.networking.v1.ClientLoginConnectionEvents;
import net.fabricmc.fabric.api.client.networking.v1.ClientPlayConnectionEvents;
import net.fabricmc.fabric.api.client.rendering.v1.hud.HudElementRegistry;
import net.fabricmc.fabric.api.client.rendering.v1.level.LevelRenderEvents;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.Minecraft;
import net.minecraft.client.User;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

public final class YakVcClient implements ClientModInitializer {
	public static final Logger LOGGER = LoggerFactory.getLogger("yakvc");

	private static @Nullable GameStateFeeder feeder;

	/** {@code config/yakvc/}: settings, per-player volumes, the client key and ticket cache, extracted natives. */
	public static Path configDir() {
		return FabricLoader.getInstance().getConfigDir().resolve("yakvc");
	}

	/** The running voice engine's feeder, or null if the engine failed to load or start. */
	public static @Nullable GameStateFeeder feeder() {
		return feeder;
	}

	@Override
	public void onInitializeClient() {
		Path configDir = configDir();
		NativeBridge bridge;
		try {
			bridge = new NativeBridge(NativeLoader.load(configDir));
		} catch (NativeLoader.UnsupportedPlatformException e) {
			disable("this platform has no native engine", e, Component.translatable("yakvc.toast.unsupported"));
			return;
		} catch (Exception | LinkageError e) {
			disable("could not load the native engine", e, Component.translatable("yakvc.toast.load_failed"));
			return;
		}
		int abi = bridge.abiVersion();
		if (abi != NativeBridge.ABI_VERSION) {
			disable("native ABI version " + abi + " does not match expected " + NativeBridge.ABI_VERSION, null,
					Component.translatable("yakvc.toast.load_failed"));
			return;
		}

		ClientConfig config;
		MemorySegment engine;
		try {
			config = ClientConfig.load(configDir);
			engine = bridge.create(configDir.toString(), config.toml());
		} catch (Exception e) {
			disable("could not start the engine", e, Component.translatable("yakvc.toast.start_failed", e.getMessage()));
			return;
		}
		LOGGER.info("Started native engine (ABI version {})", abi);
		// The dev test audio is easy to leave behind in a copied config, and it looks exactly like a silent
		// microphone and silent speakers otherwise.
		if (config.value("dev", "tone_hz") != null) {
			LOGGER.warn("[dev] tone_hz is set in client.toml: a test tone replaces the microphone");
		}
		if ("true".equals(config.value("dev", "null_output"))) {
			LOGGER.warn("[dev] null_output is set in client.toml: nothing is played");
		}

		SessionJoiner joiner = new SessionJoiner(new SessionJoiner.Mojang() {
			@Override
			public void joinServer(String serverId) throws AuthenticationException {
				Minecraft minecraft = Minecraft.getInstance();
				User user = minecraft.getUser();
				minecraft.services().sessionService().joinServer(user.getProfileId(), user.getAccessToken(), serverId);
			}

			@Override
			public boolean hasAccount() {
				Minecraft minecraft = Minecraft.getInstance();
				User user = minecraft.getUser();
				return SessionJoiner.isAccount(user.getProfileId(), user.getAccessToken(),
						minecraft.isOfflineDeveloperMode());
			}
		});
		// The game calls joinServer itself during the login phase; the configuration phase comes after it.
		ClientLoginConnectionEvents.INIT.register((handler, minecraft) -> joiner.loginStarted());
		ClientLoginConnectionEvents.DISCONNECT.register((handler, minecraft) -> joiner.loginEnded());
		ClientConfigurationConnectionEvents.INIT.register((handler, minecraft) -> joiner.loginEnded());

		GameStateFeeder feeder = new GameStateFeeder(bridge, engine, config, VoiceKeys.register(), joiner,
				PlayerVolumes.load(configDir));
		YakVcClient.feeder = feeder;
		// Authenticate with the rendezvous at the title screen, before any server is joined.
		ClientLifecycleEvents.CLIENT_STARTED.register(minecraft ->
				feeder.setIdentity(minecraft.getUser().getProfileId(), minecraft.getUser().getName()));
		ClientLifecycleEvents.CLIENT_STOPPING.register(minecraft -> feeder.close());
		ClientPlayConnectionEvents.JOIN.register((connection, sender, minecraft) -> feeder.join(connection, minecraft));
		ClientPlayConnectionEvents.DISCONNECT.register((connection, minecraft) -> feeder.leave());
		ClientTickEvents.END_CLIENT_TICK.register(feeder::tick);
		HudElementRegistry.addLast(Identifier.fromNamespaceAndPath("yakvc", "voice"), new VoiceHud(feeder));
		HudElementRegistry.addLast(Identifier.fromNamespaceAndPath("yakvc", "debug"), new DebugOverlay(feeder));
		LevelRenderEvents.COLLECT_SUBMITS.register(new TalkingIndicator(feeder)::submit);
	}

	/** Logs why voice is off for this game and tells the player once the title screen is up. */
	private static void disable(String why, @Nullable Throwable e, Component toast) {
		LOGGER.error("Voice disabled: {}", why, e);
		ClientLifecycleEvents.CLIENT_STARTED.register(minecraft -> VoiceToasts.show(toast));
	}
}
