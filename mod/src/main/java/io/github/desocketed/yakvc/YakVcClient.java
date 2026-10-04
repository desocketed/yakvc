package io.github.desocketed.yakvc;

import io.github.desocketed.yakvc.config.ClientConfig;
import io.github.desocketed.yakvc.input.VoiceKeys;
import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.natives.NativeLoader;
import io.github.desocketed.yakvc.ui.VoiceHud;
import java.lang.foreign.MemorySegment;
import java.nio.file.Path;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientLifecycleEvents;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.fabricmc.fabric.api.client.networking.v1.ClientPlayConnectionEvents;
import net.fabricmc.fabric.api.client.rendering.v1.hud.HudElementRegistry;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.resources.Identifier;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

public final class YakVcClient implements ClientModInitializer {
	public static final Logger LOGGER = LoggerFactory.getLogger("yakvc");

	private static @Nullable GameStateFeeder feeder;

	/** The running voice engine's feeder, or null if the engine failed to load or start. */
	public static @Nullable GameStateFeeder feeder() {
		return feeder;
	}

	@Override
	public void onInitializeClient() {
		Path configDir = FabricLoader.getInstance().getConfigDir().resolve("yakvc");
		NativeBridge bridge;
		try {
			bridge = new NativeBridge(NativeLoader.load(configDir));
		} catch (Exception e) {
			LOGGER.error("Voice disabled: could not load the native engine", e);
			return;
		}
		int abi = bridge.abiVersion();
		if (abi != NativeBridge.ABI_VERSION) {
			LOGGER.error("Voice disabled: native ABI version {} does not match expected {}", abi, NativeBridge.ABI_VERSION);
			return;
		}

		ClientConfig config;
		MemorySegment engine;
		try {
			config = ClientConfig.load(configDir);
			engine = bridge.create(configDir.toString(), config.toml());
		} catch (Exception e) {
			LOGGER.error("Voice disabled: could not start the engine", e);
			return;
		}
		LOGGER.info("Started native engine (ABI version {})", abi);

		GameStateFeeder feeder = new GameStateFeeder(bridge, engine, config, VoiceKeys.register());
		YakVcClient.feeder = feeder;
		// Authenticate with the rendezvous at the title screen, before any server is joined.
		ClientLifecycleEvents.CLIENT_STARTED.register(minecraft ->
				feeder.setIdentity(minecraft.getUser().getProfileId(), minecraft.getUser().getName()));
		ClientLifecycleEvents.CLIENT_STOPPING.register(minecraft -> feeder.close());
		ClientPlayConnectionEvents.JOIN.register((connection, sender, minecraft) -> feeder.join(connection, minecraft));
		ClientPlayConnectionEvents.DISCONNECT.register((connection, minecraft) -> feeder.leave());
		ClientTickEvents.END_CLIENT_TICK.register(feeder::tick);
		HudElementRegistry.addLast(Identifier.fromNamespaceAndPath("yakvc", "voice"), new VoiceHud(feeder));
	}
}
