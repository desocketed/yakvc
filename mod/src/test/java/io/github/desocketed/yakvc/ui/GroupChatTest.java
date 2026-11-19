package io.github.desocketed.yakvc.ui;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import org.junit.jupiter.api.Test;

class GroupChatTest {
	private static final long MINUTE = 60_000_000_000L;
	private static final UUID A = new UUID(0, 1);
	private static final UUID B = new UUID(0, 2);
	private static final UUID C = new UUID(0, 3);

	@Test
	void requestsRunOutAfterTwoMinutesUnlessAnswered() {
		Map<UUID, Long> asked = new HashMap<>(Map.of(A, 0L, B, 0L, C, MINUTE));
		// After one minute nothing has run out.
		assertEquals(List.of(), GroupChat.unanswered(asked, MINUTE, Set.of()));
		// B let us in: forgotten without a word. A runs out; C, asked later, not yet.
		assertEquals(List.of(A), GroupChat.unanswered(asked, 2 * MINUTE, Set.of(B)));
		assertEquals(Map.of(C, MINUTE), asked);
		// Said only once.
		assertEquals(List.of(C), GroupChat.unanswered(asked, 3 * MINUTE, Set.of()));
		assertEquals(List.of(), GroupChat.unanswered(asked, 4 * MINUTE, Set.of()));
	}
}
