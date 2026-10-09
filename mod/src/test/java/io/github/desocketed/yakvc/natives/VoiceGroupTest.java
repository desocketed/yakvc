package io.github.desocketed.yakvc.natives;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.util.List;
import java.util.UUID;
import org.junit.jupiter.api.Test;

class VoiceGroupTest {
	private static final UUID A = UUID.fromString("00112233-4455-6677-8899-aabbccddeeff");
	private static final UUID B = UUID.fromString("ffeeddcc-bbaa-9988-7766-554433221100");

	@Test
	void parsesTheList() {
		String mine = "a".repeat(64);
		String other = "b".repeat(64);
		List<VoiceGroup> groups = VoiceGroup.parse("""
				[{"id": "%s", "label": "", "public": false, "mine": true,
				  "members": ["00112233-4455-6677-8899-aabbccddeeff", "ffeeddcc-bbaa-9988-7766-554433221100"]},
				 {"id": "%s", "label": "Miners", "public": true, "mine": false,
				  "members": ["ffeeddcc-bbaa-9988-7766-554433221100"]}]
				""".formatted(mine, other));
		assertEquals(List.of(
				new VoiceGroup(mine, "", false, true, List.of(A, B)),
				new VoiceGroup(other, "Miners", true, false, List.of(B))), groups);
	}

	@Test
	void parsesAnEmptyList() {
		assertEquals(List.of(), VoiceGroup.parse("[]"));
	}
}
