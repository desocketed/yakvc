package io.github.desocketed.yakvc;

import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.natives.NativeLoader;
import java.lang.foreign.SymbolLookup;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.loader.api.FabricLoader;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

public final class YakVcClient implements ClientModInitializer {
	public static final Logger LOGGER = LoggerFactory.getLogger("yakvc");

	private static volatile boolean engineLoaded;

	/** Whether the native engine loaded and its ABI matched. */
	public static boolean engineLoaded() {
		return engineLoaded;
	}

	@Override
	public void onInitializeClient() {
		NativeBridge bridge;
		try {
			SymbolLookup library = NativeLoader.load(FabricLoader.getInstance().getConfigDir().resolve("yakvc"));
			bridge = new NativeBridge(library);
		} catch (Exception e) {
			LOGGER.error("Voice disabled: could not load the native engine", e);
			return;
		}

		int abi = bridge.abiVersion();
		if (abi != NativeBridge.ABI_VERSION) {
			LOGGER.error("Voice disabled: native ABI version {} does not match expected {}", abi, NativeBridge.ABI_VERSION);
			return;
		}
		engineLoaded = true;
		LOGGER.info("Loaded native engine (ABI version {})", abi);
	}
}
