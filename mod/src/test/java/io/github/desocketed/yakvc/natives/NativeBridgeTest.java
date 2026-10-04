package io.github.desocketed.yakvc.natives;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.nio.file.Path;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class NativeBridgeTest {
	@Test
	void loadsHostLibraryAndMatchesAbi(@TempDir Path configDir) throws Exception {
		NativeBridge bridge = new NativeBridge(NativeLoader.load(configDir));
		assertEquals(NativeBridge.ABI_VERSION, bridge.abiVersion());
	}

	@Test
	void reusesExtractedLibrary(@TempDir Path configDir) throws Exception {
		NativeLoader.load(configDir);
		NativeBridge bridge = new NativeBridge(NativeLoader.load(configDir));
		assertEquals(NativeBridge.ABI_VERSION, bridge.abiVersion());
	}
}
