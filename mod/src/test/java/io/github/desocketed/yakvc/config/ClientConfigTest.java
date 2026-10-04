package io.github.desocketed.yakvc.config;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.natives.NativeLoader;
import java.lang.foreign.MemorySegment;
import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class ClientConfigTest {
	@Test
	void writesTheDefaultOnFirstLoad(@TempDir Path dir) throws Exception {
		ClientConfig config = ClientConfig.load(dir);
		assertEquals(ClientConfig.DEFAULT, Files.readString(dir.resolve(ClientConfig.FILE_NAME)));
		assertTrue(config.respectChatRestrictions());

		Files.writeString(dir.resolve(ClientConfig.FILE_NAME), "respect_chat_restrictions = false\n");
		assertFalse(ClientConfig.load(dir).respectChatRestrictions());
	}

	@Test
	void readsOnlyTopLevelBooleans() {
		String toml = """
				# respect_chat_restrictions = false
				respect_chat_restrictions = false # opted out
				[ui]
				other = true
				""";
		assertFalse(ClientConfig.topLevelBoolean(toml, "respect_chat_restrictions", true));
		assertTrue(ClientConfig.topLevelBoolean(toml, "other", true));
		assertFalse(ClientConfig.topLevelBoolean(toml, "other", false), "keys in tables are not top-level");
	}

	@Test
	void theEngineAcceptsTheDefault(@TempDir Path dir) throws Exception {
		NativeBridge bridge = new NativeBridge(NativeLoader.load(dir));
		// Starts with real audio devices; on a machine without any that is only an engine Error event.
		MemorySegment engine = bridge.create(dir.toString(), ClientConfig.DEFAULT);
		bridge.destroy(engine);
	}
}
