package io.github.desocketed.yakvc.config;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.stream.Stream;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class AtomicFileTest {
	@Test
	void replacesTheFileAndLeavesNoTempFile(@TempDir Path dir) throws Exception {
		Path file = dir.resolve("sub").resolve("players.json");
		AtomicFile.write(file, "old");
		AtomicFile.write(file, "new");
		assertEquals("new", Files.readString(file));
		assertEquals(List.of(file), list(file.getParent()));
	}

	@Test
	void aFailedWriteLeavesTheOldContents(@TempDir Path dir) throws Exception {
		// A non-empty directory can't be replaced, so the move fails after the temp file was written.
		Path target = dir.resolve("client.toml");
		Files.createDirectories(target);
		Path inside = target.resolve("keep");
		Files.writeString(inside, "old");

		assertThrows(IOException.class, () -> AtomicFile.write(target, "new"));
		assertEquals("old", Files.readString(inside));
		assertEquals(List.of(target), list(dir));
	}

	private static List<Path> list(Path dir) throws IOException {
		try (Stream<Path> files = Files.list(dir)) {
			return files.toList();
		}
	}
}
