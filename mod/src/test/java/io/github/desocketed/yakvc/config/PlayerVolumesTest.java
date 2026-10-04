package io.github.desocketed.yakvc.config;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.UUID;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class PlayerVolumesTest {
	private static final UUID ALICE = UUID.fromString("00000000-0000-4000-8000-000000000001");
	private static final UUID BOB = UUID.fromString("00000000-0000-4000-8000-000000000002");

	@Test
	void settingsSurviveARestartByUuid(@TempDir Path dir) throws Exception {
		PlayerVolumes volumes = PlayerVolumes.load(dir);
		assertEquals(PlayerVolumes.DEFAULT, volumes.get(ALICE));
		volumes.set(ALICE, new PlayerVolumes.Setting(1.5f, false));
		volumes.set(BOB, new PlayerVolumes.Setting(1f, true));
		volumes.save();

		PlayerVolumes reloaded = PlayerVolumes.load(dir);
		assertEquals(new PlayerVolumes.Setting(1.5f, false), reloaded.get(ALICE));
		assertEquals(new PlayerVolumes.Setting(1f, true), reloaded.get(BOB));
	}

	@Test
	void defaultsAreNotStored(@TempDir Path dir) throws Exception {
		PlayerVolumes volumes = PlayerVolumes.load(dir);
		volumes.set(ALICE, new PlayerVolumes.Setting(0.5f, false));
		volumes.set(ALICE, PlayerVolumes.DEFAULT);
		volumes.save();
		assertFalse(Files.readString(dir.resolve(PlayerVolumes.FILE_NAME)).contains(ALICE.toString()));
	}

	@Test
	void aBrokenFileStartsEmpty(@TempDir Path dir) throws Exception {
		Files.writeString(dir.resolve(PlayerVolumes.FILE_NAME), "{not json");
		assertEquals(PlayerVolumes.DEFAULT, PlayerVolumes.load(dir).get(ALICE));
	}
}
