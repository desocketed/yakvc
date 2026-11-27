package io.github.desocketed.yakvc.config;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.NoSuchFileException;
import java.nio.file.Path;
import java.nio.file.attribute.FileTime;
import java.util.Objects;
import org.jspecify.annotations.Nullable;

/** Notices {@code settings.json} changing on disk, so an edit by hand applies while the game runs. */
public final class SettingsWatch {
	private final Path file;
	private @Nullable FileTime seen;

	public SettingsWatch(Path configDir) {
		file = configDir.resolve(Settings.FILE_NAME);
		seen = modified();
	}

	/** The settings in the file if it changed since the last call, else null. */
	public @Nullable Settings changed() throws IOException {
		FileTime now = modified();
		if (Objects.equals(now, seen)) return null;
		seen = now;
		return now == null ? null : Settings.parse(Files.readString(file));
	}

	private @Nullable FileTime modified() {
		try {
			return Files.getLastModifiedTime(file);
		} catch (NoSuchFileException e) {
			return null;
		} catch (IOException e) {
			return seen;
		}
	}
}
