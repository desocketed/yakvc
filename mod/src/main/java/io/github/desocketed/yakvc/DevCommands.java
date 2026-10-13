package io.github.desocketed.yakvc;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import net.minecraft.client.Minecraft;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Commands fed to the client from a file, for dev sessions only: each line appended to {@code yakvc-commands.txt}
 * in the game directory is sent as if typed in chat (without the leading slash), so a tool watching the session
 * from outside the game can change the world while someone plays. Lines already in the file when the game started
 * are skipped, so a restart doesn't replay them.
 */
final class DevCommands {
	private static final Logger LOGGER = LoggerFactory.getLogger("yakvc");
	static final String FILE_NAME = "yakvc-commands.txt";

	private final Path file;
	private int sent = -1;

	DevCommands(Path gameDir) {
		this.file = gameDir.resolve(FILE_NAME);
	}

	void tick(Minecraft minecraft) {
		if (minecraft.player == null || !Files.exists(file)) return;
		List<String> lines;
		try {
			lines = Files.readAllLines(file);
		} catch (IOException e) {
			return; // Half-written; next tick.
		}
		if (sent < 0) sent = lines.size();
		if (lines.size() < sent) sent = 0; // The file was truncated and started over.
		for (; sent < lines.size(); sent++) {
			String line = lines.get(sent).strip();
			if (line.isEmpty() || line.startsWith("#")) continue;
			if (line.startsWith("/")) line = line.substring(1);
			LOGGER.info("Dev command: /{}", line);
			minecraft.player.connection.sendCommand(line);
		}
	}
}
