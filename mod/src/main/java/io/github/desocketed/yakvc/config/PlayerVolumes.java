package io.github.desocketed.yakvc.config;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonParseException;
import com.google.gson.reflect.TypeToken;
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
	private static final Gson GSON = new GsonBuilder().setPrettyPrinting().create();

	/** {@code volume} is a gain: 1 is unchanged, 2 is twice as loud. */
	public record Setting(float volume, boolean muted) {}

	private final Path file;
	private final Map<UUID, Setting> settings;

	private PlayerVolumes(Path file, Map<UUID, Setting> settings) {
		this.file = file;
		this.settings = settings;
	}

	/** Reads the saved settings. A missing or unreadable file starts empty rather than turning voice off. */
	public static PlayerVolumes load(Path configDir) {
		Path file = configDir.resolve(FILE_NAME);
		Map<UUID, Setting> settings = new TreeMap<>();
		if (Files.exists(file)) {
			try {
				Map<UUID, Setting> saved = GSON.fromJson(Files.readString(file),
						new TypeToken<Map<UUID, Setting>>() {}.getType());
				if (saved != null) settings.putAll(saved);
			} catch (IOException | JsonParseException e) {
				YakVcClient.LOGGER.warn("Ignoring unreadable {}", file, e);
			}
		}
		return new PlayerVolumes(file, settings);
	}

	public Setting get(UUID player) {
		return settings.getOrDefault(player, DEFAULT);
	}

	public void set(UUID player, Setting setting) {
		if (setting.equals(DEFAULT)) settings.remove(player);
		else settings.put(player, setting);
	}

	public void save() throws IOException {
		Files.createDirectories(file.getParent());
		Files.writeString(file, GSON.toJson(settings));
	}
}
