package io.github.desocketed.yakvc.config;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * {@code config/yakvc/client.toml}. The engine parses the whole file itself; Java passes it over as a string and
 * reads only its own top-level settings here.
 */
public record ClientConfig(String toml, boolean respectChatRestrictions) {
	public static final String FILE_NAME = "client.toml";

	/** Reads the config, writing the commented default first if there is none. */
	public static ClientConfig load(Path configDir) throws IOException {
		Path path = configDir.resolve(FILE_NAME);
		if (!Files.exists(path)) {
			Files.createDirectories(configDir);
			Files.writeString(path, DEFAULT);
		}
		return parse(Files.readString(path));
	}

	public static ClientConfig parse(String toml) {
		return new ClientConfig(toml, topLevelBoolean(toml, "respect_chat_restrictions", true));
	}

	/**
	 * Finds {@code key = true|false} before the first {@code [table]}. A full TOML parser would be a new dependency
	 * for two booleans; the engine rejects malformed files anyway.
	 */
	static boolean topLevelBoolean(String toml, String key, boolean fallback) {
		for (String line : toml.split("\n")) {
			String code = line.split("#", 2)[0].strip();
			if (code.startsWith("[")) break;
			String[] parts = code.split("=", 2);
			if (parts.length == 2 && parts[0].strip().equals(key)) {
				return switch (parts[1].strip()) {
					case "true" -> true;
					case "false" -> false;
					default -> fallback;
				};
			}
		}
		return fallback;
	}

	/** Written on first start. */
	static final String DEFAULT = """
			# Yak VC client settings. Changes apply on the next game start.

			# Voice range in blocks (1 to 256). Between two players the shorter range applies.
			voice_range = 48.0

			# Never use direct connections: all voice goes through the relay, so peers don't learn your IP address.
			relay_only = false

			# Turn voice off while your Microsoft account or launcher disables chat.
			respect_chat_restrictions = true

			# Development against a local `yakvc-server` with `insecure_dev_auth = true`: uncomment these two lines
			# and the [rendezvous] table below, filled in with the IDs the server prints at startup.
			# dev_mode = true
			# trusted_issuers = ["<issuer id>"]

			[audio]
			# "push_to_talk" or "voice".
			activation = "push_to_talk"
			# Opus bitrate in bits per second (16000 to 64000).
			bitrate = 24000
			noise_suppression = true
			# Unset means the system default microphone, and speakers matching the game's sound device.
			# input_device = "..."
			# output_device = "..."

			# [rendezvous]
			# endpoint_id = "<endpoint id>"
			# addrs = ["127.0.0.1:4433"]

			# Dev mode only: a test tone instead of the microphone, and no speakers, for machines without audio.
			# [dev]
			# tone_hz = 440.0
			# null_output = true
			""";
}
