package io.github.desocketed.yakvc.config;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import org.jspecify.annotations.Nullable;

/**
 * {@code config/yakvc/client.toml}. The engine parses the whole file itself; Java passes it over as a string, reads
 * the few values the settings screen shows, and edits them line by line so the user's comments and other keys stay.
 *
 * <p>A full TOML library would be a new dependency for a handful of plain keys; the engine rejects malformed files
 * anyway, and every edit is checked by the engine before it is saved.
 */
public record ClientConfig(String toml) {
	public static final String FILE_NAME = "client.toml";

	/** Reads the config, writing the commented default first if there is none. */
	public static ClientConfig load(Path configDir) throws IOException {
		Path path = configDir.resolve(FILE_NAME);
		if (!Files.exists(path)) {
			Files.createDirectories(configDir);
			Files.writeString(path, DEFAULT);
		}
		return new ClientConfig(Files.readString(path));
	}

	public void save(Path configDir) throws IOException {
		Files.createDirectories(configDir);
		Files.writeString(configDir.resolve(FILE_NAME), toml);
	}

	public boolean respectChatRestrictions() {
		return bool(value("", "respect_chat_restrictions"), true);
	}

	public boolean muteBlockedPlayers() {
		return bool(value("", "mute_blocked_players"), true);
	}

	public boolean relayOnly() {
		return bool(value("", "relay_only"), false);
	}

	public boolean verifiedOnly() {
		return bool(value("", "verified_only"), false);
	}

	/** Blocks; the engine's default when unset. */
	public double voiceRange() {
		return number(value("", "voice_range"), 48);
	}

	public boolean voiceActivation() {
		return "voice".equals(string(value("audio", "activation")));
	}

	/** dBFS above which voice activation transmits; the engine's default when unset. */
	public double vadThresholdDb() {
		return number(value("audio", "vad_threshold_db"), -45);
	}

	/** Bits per second. */
	public int bitrate() {
		return (int) number(value("audio", "bitrate"), 24_000);
	}

	/** The microphone's name, or null for the system default. */
	public @Nullable String inputDevice() {
		return string(value("audio", "input_device"));
	}

	/** The speakers' name, or null to follow the game's sound device. */
	public @Nullable String outputDevice() {
		return string(value("audio", "output_device"));
	}

	/**
	 * The raw TOML value of {@code key} in {@code table} ("" for the top level), comment stripped, or null if unset.
	 */
	public @Nullable String value(String table, String key) {
		String current = "";
		for (String line : toml.split("\n", -1)) {
			String code = stripComment(line).strip();
			if (code.startsWith("[")) {
				current = tableName(code);
			} else if (current.equals(table) && keyOf(code).equals(key)) {
				return code.substring(code.indexOf('=') + 1).strip();
			}
		}
		return null;
	}

	/**
	 * Returns a copy with {@code key = value} in {@code table} ("" for the top level). {@code value} is a TOML literal
	 * such as {@code true}, {@code 48.0} or {@link #quote}d text; null removes the key. A commented-out line for the
	 * key ({@code # input_device = "..."}) is replaced in place, so the setting stays next to its explanation.
	 */
	public ClientConfig with(String table, String key, @Nullable String value) {
		List<String> lines = new ArrayList<>(List.of(toml.split("\n", -1)));
		// The table each line belongs to; a header belongs to its own table.
		List<String> tables = new ArrayList<>();
		String current = "";
		for (String line : lines) {
			String code = stripComment(line).strip();
			if (code.startsWith("[")) current = tableName(code);
			tables.add(current);
		}
		String newLine = key + " = " + value;

		for (int i = 0; i < lines.size(); i++) {
			if (tables.get(i).equals(table) && keyOf(stripComment(lines.get(i)).strip()).equals(key)) {
				if (value == null) lines.remove(i);
				else lines.set(i, newLine);
				return new ClientConfig(String.join("\n", lines));
			}
		}
		if (value == null) return this;
		for (int i = 0; i < lines.size(); i++) {
			if (tables.get(i).equals(table) && stripComment(lines.get(i)).isBlank()
					&& keyOf(uncomment(lines.get(i))).equals(key)) {
				lines.set(i, newLine);
				return new ClientConfig(String.join("\n", lines));
			}
		}
		// After the table's last setting or its header, or else in a new table at the end.
		int last = table.isEmpty() ? -1 : -2;
		for (int i = 0; i < lines.size(); i++) {
			if (tables.get(i).equals(table) && !stripComment(lines.get(i)).isBlank()) last = i;
		}
		if (last == -2) {
			if (!lines.getLast().isBlank()) lines.add("");
			lines.add("[" + table + "]");
			lines.add(newLine);
			lines.add("");
		} else {
			lines.add(last + 1, newLine);
		}
		return new ClientConfig(String.join("\n", lines));
	}

	/** A TOML basic string. */
	public static String quote(String text) {
		return '"' + text.replace("\\", "\\\\").replace("\"", "\\\"") + '"';
	}

	private static boolean bool(@Nullable String value, boolean fallback) {
		if ("true".equals(value)) return true;
		if ("false".equals(value)) return false;
		return fallback;
	}

	private static double number(@Nullable String value, double fallback) {
		if (value == null) return fallback;
		try {
			return Double.parseDouble(value.replace("_", ""));
		} catch (NumberFormatException e) {
			return fallback;
		}
	}

	/** The text of a basic or literal string, or null if {@code value} is not one. */
	static @Nullable String string(@Nullable String value) {
		if (value == null || value.length() < 2) return null;
		if (value.startsWith("'") && value.endsWith("'")) return value.substring(1, value.length() - 1);
		if (!value.startsWith("\"") || !value.endsWith("\"")) return null;
		StringBuilder out = new StringBuilder();
		for (int i = 1; i < value.length() - 1; i++) {
			char c = value.charAt(i);
			if (c == '\\' && i + 1 < value.length() - 1) {
				c = value.charAt(++i);
				out.append(switch (c) {
					case 'n' -> '\n';
					case 't' -> '\t';
					default -> c;
				});
			} else {
				out.append(c);
			}
		}
		return out.toString();
	}

	/** The line up to a {@code #} that is not inside a string. */
	private static String stripComment(String line) {
		char quote = 0;
		for (int i = 0; i < line.length(); i++) {
			char c = line.charAt(i);
			if (quote == '"' && c == '\\') i++;
			else if (quote != 0 && c == quote) quote = 0;
			else if (quote == 0 && (c == '"' || c == '\'')) quote = c;
			else if (quote == 0 && c == '#') return line.substring(0, i);
		}
		return line;
	}

	/** {@code # key = value} becomes {@code key = value}. */
	private static String uncomment(String line) {
		String s = line.strip();
		return s.startsWith("#") ? s.substring(1).strip() : "";
	}

	/** The key of a {@code key = value} line, or "". */
	private static String keyOf(String code) {
		int eq = code.indexOf('=');
		return eq < 0 ? "" : code.substring(0, eq).strip();
	}

	private static String tableName(String header) {
		int end = header.indexOf(']');
		return header.substring(1, end < 0 ? header.length() : end).strip();
	}

	/** Written on first start. */
	static final String DEFAULT = """
			# Yak VC client settings. The voice menu's settings screen edits this file too; relay_only and the
			# [rendezvous] table apply on the next game start, everything else at once.

			# Voice range in blocks (1 to 256). Between two players the shorter range applies.
			voice_range = 48.0

			# Never use direct connections: all voice goes through the relay, so peers don't learn your IP address.
			relay_only = false

			# Turn voice off while your Microsoft account or launcher disables chat.
			respect_chat_restrictions = true

			# Mute players you blocked in Social Interactions, both ways.
			mute_blocked_players = true

			# Only talk with players whose Minecraft account is verified with Mojang. Players without an account
			# (on offline-mode servers) are then neither heard nor sent your voice.
			verified_only = false

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

			# Without a [rendezvous] table Yak VC uses its built-in server (except with dev_mode). For your own
			# server, uncomment it and list the server's issuer in trusted_issuers above.
			# [rendezvous]
			# endpoint_id = "<endpoint id>"
			# addrs = ["127.0.0.1:4433"]

			# Dev mode only: a test tone instead of the microphone, and no speakers, for machines without audio.
			# [dev]
			# tone_hz = 440.0
			# null_output = true
			""";
}
