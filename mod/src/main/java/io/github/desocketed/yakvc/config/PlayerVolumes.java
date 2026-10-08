package io.github.desocketed.yakvc.config;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import io.github.desocketed.yakvc.YakVcClient;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import java.util.TreeMap;
import java.util.UUID;

/**
 * Per-player volume and mute chosen in the voice menu, kept by UUID in {@code config/yakvc/players.json} so they
 * survive name changes and restarts. Players at the default (full volume, not muted) are not stored.
 */
public final class PlayerVolumes {
	public static final String FILE_NAME = "players.json";
	public static final Setting DEFAULT = new Setting(1f, false);
	/** The voice menu's slider goes up to 200%. */
	public static final float MAX_VOLUME = 2f;
	private static final Gson GSON = new GsonBuilder().setPrettyPrinting().create();

	/** {@code volume} is a gain: 1 is unchanged, 2 is twice as loud. */
	public record Setting(float volume, boolean muted) {}

	private final Path file;
	private final Map<UUID, Setting> settings;

	private PlayerVolumes(Path file, Map<UUID, Setting> settings) {
		this.file = file;
		this.settings = settings;
	}

	/**
	 * Reads the saved settings. A missing or unreadable file starts empty rather than turning voice off. The file may
	 * be edited by hand, so each entry is checked on its own: a broken one is dropped, and volumes are clamped to what
	 * the menu can set, because the engine rejects a negative or non-finite volume.
	 */
	public static PlayerVolumes load(Path configDir) {
		Path file = configDir.resolve(FILE_NAME);
		Map<UUID, Setting> settings = new TreeMap<>();
		if (Files.exists(file)) {
			try {
				JsonObject saved = JsonParser.parseString(Files.readString(file)).getAsJsonObject();
				for (Map.Entry<String, JsonElement> entry : saved.entrySet()) {
					try {
						settings.put(UUID.fromString(entry.getKey()), parse(entry.getValue().getAsJsonObject()));
					} catch (RuntimeException e) {
						YakVcClient.LOGGER.warn("Ignoring the entry {} in {}: {}", entry.getKey(), file, e.toString());
					}
				}
			} catch (IOException | RuntimeException e) {
				YakVcClient.LOGGER.warn("Ignoring unreadable {}", file, e);
			}
		}
		return new PlayerVolumes(file, settings);
	}

	/** A missing field keeps its default. */
	private static Setting parse(JsonObject json) {
		float volume = json.has("volume") ? json.get("volume").getAsFloat() : DEFAULT.volume();
		boolean muted = json.has("muted") ? json.get("muted").getAsBoolean() : DEFAULT.muted();
		// NaN fails every comparison, so it can't be clamped.
		if (Float.isNaN(volume)) throw new IllegalArgumentException("volume is not a number");
		return new Setting(Math.clamp(volume, 0f, MAX_VOLUME), muted);
	}

	public Setting get(UUID player) {
		return settings.getOrDefault(player, DEFAULT);
	}

	public void set(UUID player, Setting setting) {
		if (setting.equals(DEFAULT)) settings.remove(player);
		else settings.put(player, setting);
	}

	public void save() throws IOException {
		AtomicFile.write(file, GSON.toJson(settings));
	}
}
