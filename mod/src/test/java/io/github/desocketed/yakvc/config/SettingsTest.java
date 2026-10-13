package io.github.desocketed.yakvc.config;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import org.junit.jupiter.api.Test;
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
				.withInputDevice("Mic \"USB\" \\ 2")
				.withAcoustic("furnishing", 0.9);
		settings = Settings.parse("""
				{"dev_mode": true, "trusted_issuers": ["abc"], "friends_only": ["00000000-0000-0000-0000-000000000001"],
				 "rendezvous": {"endpoint_id": "def", "addrs": ["127.0.0.1:4433"], "relay": "https://relay.example/"},
				 "dev": {"tone_hz": 440, "null_output": true}}
				""").withOutputDevice(settings.audio().inputDevice()).withInputGain(settings.audio().inputGain());
		settings.save(dir);
		Settings loaded = Settings.load(dir);
		assertEquals(settings, loaded);
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
}
