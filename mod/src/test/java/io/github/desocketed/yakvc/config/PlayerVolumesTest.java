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
	void handEditedEntriesAreCheckedOneByOne(@TempDir Path dir) throws Exception {
		UUID carol = UUID.fromString("00000000-0000-4000-8000-000000000003");
		UUID dave = UUID.fromString("00000000-0000-4000-8000-000000000004");
		UUID erin = UUID.fromString("00000000-0000-4000-8000-000000000005");
		UUID frank = UUID.fromString("00000000-0000-4000-8000-000000000006");
		Files.writeString(dir.resolve(PlayerVolumes.FILE_NAME), """
				{
				  "not-a-uuid": {"volume": 0.5, "muted": false},
				  "%s": null,
				  "%s": {"volume": -1, "muted": true},
				  "%s": {"volume": 1e999},
				  "%s": {"volume": NaN, "muted": true},
				  "%s": {"volume": "loud"},
				  "%s": {"volume": 0.25, "muted": false}
				}
				""".formatted(ALICE, BOB, carol, dave, erin, frank));

		PlayerVolumes volumes = PlayerVolumes.load(dir);
		assertEquals(PlayerVolumes.DEFAULT, volumes.get(ALICE), "a null entry is dropped");
		assertEquals(new PlayerVolumes.Setting(0f, true), volumes.get(BOB), "a negative volume is clamped");
		assertEquals(new PlayerVolumes.Setting(PlayerVolumes.MAX_VOLUME, false), volumes.get(carol),
				"an infinite volume is clamped and a missing field keeps its default");
		assertEquals(PlayerVolumes.DEFAULT, volumes.get(dave), "NaN is dropped");
		assertEquals(PlayerVolumes.DEFAULT, volumes.get(erin), "a malformed entry is dropped");
		assertEquals(new PlayerVolumes.Setting(0.25f, false), volumes.get(frank), "good entries are kept");
	}

	@Test
	void aBrokenFileStartsEmpty(@TempDir Path dir) throws Exception {
		Files.writeString(dir.resolve(PlayerVolumes.FILE_NAME), "{not json");
		assertEquals(PlayerVolumes.DEFAULT, PlayerVolumes.load(dir).get(ALICE));
	}
}
