package io.github.desocketed.yakvc.config;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.natives.NativeLoader;
import io.github.desocketed.yakvc.natives.YakVcException;
import java.lang.foreign.MemorySegment;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.UUID;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class ClientConfigTest {
	@Test
	void writesTheDefaultOnFirstLoad(@TempDir Path dir) throws Exception {
		ClientConfig config = ClientConfig.load(dir);
		assertEquals(ClientConfig.DEFAULT, Files.readString(dir.resolve(ClientConfig.FILE_NAME)));
		assertTrue(config.respectChatRestrictions());
		assertTrue(config.muteBlockedPlayers());

		Files.writeString(dir.resolve(ClientConfig.FILE_NAME), "respect_chat_restrictions = false\n");
		assertFalse(ClientConfig.load(dir).respectChatRestrictions());
	}

	@Test
	void readsValuesFromTheirTable() {
		ClientConfig config = new ClientConfig("""
				# respect_chat_restrictions = true
				respect_chat_restrictions = false # opted out
				[ui]
				mute_blocked_players = false
				[audio]
				input_device = "Mic \\"USB\\" # 2" # a comment
				output_device = 'C:\\Speakers'
				""");
		assertFalse(config.respectChatRestrictions());
		assertTrue(config.muteBlockedPlayers(), "keys in tables are not top-level");
		assertEquals("Mic \"USB\" # 2", config.inputDevice());
		assertEquals("C:\\Speakers", config.outputDevice());
		assertEquals(48, config.voiceRange(), "the engine's default when unset");
	}

	@Test
	void editsKeepCommentsAndOtherKeys() {
		ClientConfig config = new ClientConfig(ClientConfig.DEFAULT)
				.with("", "voice_range", "32.0")
				.with("", "relay_only", "true")
				.with("audio", "activation", ClientConfig.quote("voice"))
				.with("audio", "input_device", ClientConfig.quote("Mic \"2\""));
		assertEquals(32, config.voiceRange());
		assertTrue(config.relayOnly());
		assertTrue(config.voiceActivation());
		assertEquals("Mic \"2\"", config.inputDevice());
		assertTrue(config.toml().contains("# Voice range in blocks"));
		assertTrue(config.toml().contains("# output_device = \"...\""), "untouched commented keys stay");
		assertFalse(config.toml().contains("# input_device"), "the commented line was replaced in place");
		assertEquals(ClientConfig.DEFAULT.lines().count(), config.toml().lines().count());

		ClientConfig unset = config.with("audio", "input_device", null);
		assertNull(unset.inputDevice());
		assertSame(unset, unset.with("audio", "input_device", null), "removing a missing key changes nothing");
	}

	@Test
	void editsAddMissingKeysAndTables() {
		ClientConfig config = new ClientConfig("dev_mode = true\n[dev]\nnull_output = true\n")
				.with("", "mute_blocked_players", "false")
				.with("audio", "bitrate", "32000");
		assertEquals("dev_mode = true\nmute_blocked_players = false\n[dev]\nnull_output = true\n\n[audio]\nbitrate = 32000\n",
				config.toml());
		assertFalse(config.muteBlockedPlayers());
		assertEquals(32000, config.bitrate());
		assertEquals("voice_range = 8.0", new ClientConfig("").with("", "voice_range", "8.0").toml().strip());
	}

	@Test
	void theEngineAcceptsTheDefaultAndEditsAndRejectsBadValues(@TempDir Path dir) throws Exception {
		NativeBridge bridge = new NativeBridge(NativeLoader.load(dir));
		// Starts with real audio devices; on a machine without any that is only an engine Error event.
		MemorySegment engine = bridge.create(dir.toString(), ClientConfig.DEFAULT);
		try {
			ClientConfig edited = new ClientConfig(ClientConfig.DEFAULT)
					.with("", "voice_range", "100.0")
					.with("", "mute_blocked_players", "false")
					.with("audio", "bitrate", "64000")
					.with("audio", "output_device", ClientConfig.quote("Speakers"));
			bridge.updateConfig(engine, edited.toml());
			assertThrows(YakVcException.class,
					() -> bridge.updateConfig(engine, edited.with("", "voice_range", "1000.0").toml()));
			// Java keeps per-player volume; the engine only checks it is a usable gain.
			bridge.setPeerVolume(engine, UUID.randomUUID(), 2f, true);
		} finally {
			bridge.destroy(engine);
		}
	}
}
