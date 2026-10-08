package io.github.desocketed.yakvc.config;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;

/**
 * Saves a file so that a crash mid-write leaves the old contents, not a truncated file: a truncated {@code client.toml}
 * stops voice from starting, and a truncated {@code players.json} loads as empty and the next save loses every mute.
 */
final class AtomicFile {
	private AtomicFile() {}

	/** Writes {@code text} to a temporary file beside {@code path}, then moves it over {@code path} in one step. */
	static void write(Path path, String text) throws IOException {
		Files.createDirectories(path.getParent());
		Path tmp = Files.createTempFile(path.getParent(), path.getFileName().toString(), ".tmp");
		try {
			Files.writeString(tmp, text);
			Files.move(tmp, path, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
		} finally {
			Files.deleteIfExists(tmp);
		}
	}
}
