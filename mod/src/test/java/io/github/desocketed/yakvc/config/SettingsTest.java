package io.github.desocketed.yakvc.config;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.natives.NativeLoader;
import io.github.desocketed.yakvc.natives.YakVcException;
import java.io.IOException;
import java.lang.foreign.MemorySegment;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.CleanupMode;
import org.junit.jupiter.api.io.TempDir;

class SettingsTest {
	@Test
	void writesTheDefaultsOnFirstLoad(@TempDir Path dir) throws Exception {
		assertEquals(Settings.DEFAULT, Settings.load(dir));
		JsonObject written = JsonParser.parseString(Files.readString(dir.resolve(Settings.FILE_NAME))).getAsJsonObject();
		assertEquals(256, written.get("voice_range").getAsDouble());
		assertEquals("simple", written.get("spatial_audio").getAsString());
		assertEquals(24_000, written.getAsJsonObject("audio").get("bitrate").getAsInt());
		assertTrue(written.getAsJsonObject("audio").get("input_device").isJsonNull(), "unset keys are shown");
		assertEquals(0.35, written.getAsJsonObject("acoustics").get("reverb_level").getAsDouble());
		assertEquals(Settings.DEFAULT, Settings.load(dir), "the written file reads back as the defaults");
	}

	@Test
	void aPartialFileIsAPreset() throws Exception {
		Settings preset = Settings.parse("""
				{"setup_done": true, "check_for_updates": false, "audio": {"bitrate": 32000}, "hud_corner": "top_left"}
				""");
		assertTrue(preset.setupDone());
		assertFalse(preset.checkForUpdates());
		assertEquals(32_000, preset.audio().bitrate());
		assertEquals(Settings.DEFAULT.audio().inputGain(), preset.audio().inputGain(), "the rest of audio keeps its defaults");
		assertEquals(Settings.DEFAULT.acoustics(), preset.acoustics());
		assertEquals(Settings.DEFAULT, preset.withSetupDone(false).withCheckForUpdates(true).withBitrate(24_000),
				"unknown keys are dropped");
	}

	@Test
	void savesAndLoadsEveryKind(@TempDir Path dir) throws Exception {
		Settings settings = Settings.DEFAULT
				.withVoiceRange(32)
				.withSpatialAudio(SpatialAudio.FULL)
				.withInvites("friends")
				.withActivation("voice")
				.withInputGain(2.5)
				.withVadThresholdDb(-30)
				.withInputDevice("Mic \"USB\" \\ 2")
				.withAcoustic("furnishing", 0.9);
		settings = Settings.parse("""
				{"dev_mode": true, "trusted_issuers": ["abc"], "friends_only": ["00000000-0000-0000-0000-000000000001"],
				 "rendezvous": {"endpoint_id": "def", "addrs": ["127.0.0.1:4433"], "relay": "https://relay.example/"},
				 "dev": {"tone_hz": 440, "null_output": true}}
				""").withOutputDevice(settings.audio().inputDevice()).withInputGain(settings.audio().inputGain())
				.withVadThresholdDb(settings.audio().vadThresholdDb());
		settings.save(dir);
		Settings loaded = Settings.load(dir);
		assertEquals(settings, loaded);
		assertEquals(-30, loaded.audio().vadThresholdDb());
		assertTrue(loaded.devMode());
		assertEquals(List.of("abc"), loaded.trustedIssuers());
		assertEquals(new Settings.Rendezvous("def", List.of("127.0.0.1:4433"), "https://relay.example/"),
				loaded.rendezvous());
		assertEquals(new Settings.Dev(440.0, null, true), loaded.dev());
		assertEquals("Mic \"USB\" \\ 2", loaded.audio().outputDevice());
		assertNull(loaded.withOutputDevice(null).audio().outputDevice());
	}

	@Test
	void badFilesAreRefusedWithTheirReason() {
		IOException level = assertThrows(IOException.class, () -> Settings.parse("{\"spatial_audio\": \"loud\"}"));
		assertTrue(level.getMessage().contains("spatial_audio"), level.getMessage());
		IOException syntax = assertThrows(IOException.class, () -> Settings.parse("{\"voice_range\": }"));
		assertTrue(syntax.getMessage().startsWith(Settings.FILE_NAME), syntax.getMessage());
		assertThrows(IOException.class, () -> Settings.parse("{\"voice_range\": \"far\"}"));
		assertThrows(IllegalArgumentException.class, () -> Settings.DEFAULT.withAcoustic("reverb", 1));
	}

	@Test
	void voiceRangeIsCappedByRenderDistance() {
		assertEquals(128, Settings.effectiveVoiceRange(256, 8));
		assertEquals(100, Settings.effectiveVoiceRange(100, 8));
		assertEquals(512, Settings.effectiveVoiceRange(512, 32));
	}

	@Test
	void engineTomlHasTheEngineKeysQuotedAndCapped() throws Exception {
		String toml = Settings.DEFAULT.engineToml(0);
		assertTrue(toml.contains("voice_range = 256.0\n"), toml);
		assertTrue(toml.contains("trusted_issuers = []\n"), toml);
		assertTrue(toml.contains("\n[audio]\nactivation = \"push_to_talk\"\nbitrate = 24000\n"), toml);
		assertTrue(toml.contains("vad_threshold_db = -45.0\n"), toml);
		assertTrue(toml.contains("\n[acoustics]\nfull_volume_distance = 4.0\n"), toml);
		assertFalse(toml.contains("input_device"), "unset devices are left out");
		assertFalse(toml.contains("[dev]") || toml.contains("[rendezvous]"), toml);
		assertFalse(toml.contains("setup_done") || toml.contains("spatial_audio") || toml.contains("furnishing"),
				"the mod's own keys stay out");
		assertTrue(Settings.DEFAULT.engineToml(8).contains("voice_range = 128.0\n"));

		Settings full = Settings.parse("""
				{"dev_mode": true, "trusted_issuers": ["abc"], "friends_only": ["00000000-0000-0000-0000-000000000001"],
				 "rendezvous": {"endpoint_id": "def", "addrs": ["127.0.0.1:4433", "[::1]:4433"], "relay": "https://r/"},
				 "dev": {"wav": "C:\\\\talk\\\\a.wav", "null_output": true}}
				""").withInputDevice("Mic \"USB\"\t2").withOutputDevice("Speakers\n");
		toml = full.engineToml(0);
		assertTrue(toml.contains("trusted_issuers = [\"abc\"]\n"), toml);
		assertTrue(toml.contains("friends_only = [\"00000000-0000-0000-0000-000000000001\"]\n"), toml);
		assertTrue(toml.contains("input_device = \"Mic \\\"USB\\\"\\t2\"\n"), toml);
		assertTrue(toml.contains("output_device = \"Speakers\\n\"\n"), toml);
		assertTrue(toml.contains("\n[rendezvous]\nendpoint_id = \"def\"\naddrs = [\"127.0.0.1:4433\", \"[::1]:4433\"]\n"
				+ "relay = \"https://r/\"\n"), toml);
		assertTrue(toml.contains("\n[dev]\nwav = \"C:\\\\talk\\\\a.wav\"\nnull_output = true\n"), toml);
		assertEquals("\"\\u0001\"", Settings.quote("\u0001"));
	}

	@Test
	// Never cleaned up: Windows cannot delete the still loaded library.
	void theEngineTakesTheTomlAndRejectsBadValues(@TempDir(cleanup = CleanupMode.NEVER) Path dir) throws Exception {
		NativeBridge bridge = new NativeBridge(NativeLoader.load(dir));
		// Starts with real audio devices; on a machine without any that is only an engine Error event.
		MemorySegment engine = bridge.create(dir.toString(), Settings.DEFAULT.engineToml(0));
		try {
			// The built-in server's ids, which parse as any valid ones do.
			Settings edited = Settings.parse("""
					{"dev_mode": true,
					 "trusted_issuers": ["7e2624000ff23e1db237755c4e1f79ff29d78d314e66bf9eb5654a74b75bd20f"],
					 "friends_only": ["00000000-0000-0000-0000-000000000001"],
					 "rendezvous": {"endpoint_id": "5c9578695b7b258c587bebc826f1091a95da2e75ef93a4b7aeff45f28e2cde60",
					                "addrs": ["127.0.0.1:4433"], "relay": "https://relay.example.com/"},
					 "dev": {"tone_hz": 440, "null_output": true}}
					""")
					.withVoiceRange(100)
					.withSpatialAudio(SpatialAudio.FULL)
					.withInvites("friends")
					.withActivation("voice")
					.withBitrate(64_000)
					.withInputGain(4)
					.withOutputDevice("Speakers \"2\"")
					.withAcoustic("reverb_level", 0.5);
			bridge.updateConfig(engine, edited.engineToml(8));
			assertThrows(YakVcException.class, () -> bridge.updateConfig(engine, edited.withVoiceRange(1000).engineToml(0)));
			assertThrows(YakVcException.class, () -> bridge.updateConfig(engine, edited.withInputGain(20).engineToml(0)));
			assertThrows(YakVcException.class, () -> bridge.updateConfig(engine, edited.withVadThresholdDb(3).engineToml(0)));
			assertThrows(YakVcException.class, () -> bridge.updateConfig(engine, edited.withInvites("all").engineToml(0)));
		} finally {
			bridge.destroy(engine);
		}
	}
}
