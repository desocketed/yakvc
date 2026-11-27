package io.github.desocketed.yakvc;

import static org.junit.jupiter.api.Assertions.assertEquals;

import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.suggestion.Suggestion;
import io.github.desocketed.yakvc.heads.HeadStyle;
import java.util.ArrayList;
import java.util.List;
import org.jspecify.annotations.Nullable;
import org.junit.jupiter.api.Test;

class YakVcCommandTest {
	private final List<String> ran = new ArrayList<>();
	private final CommandDispatcher<Object> dispatcher = new CommandDispatcher<>();

	YakVcCommandTest() {
		dispatcher.register(YakVcCommand.tree(new YakVcCommand.Actions<>() {
			@Override
			public int settings() {
				ran.add("settings");
				return 1;
			}

			@Override
			public int menu() {
				ran.add("menu");
				return 1;
			}

			@Override
			public int heads(Object source, HeadStyle style, @Nullable Double strength) {
				ran.add(style.key() + " " + strength);
				return 1;
			}
		}));
	}

	private List<String> complete(String input) {
		return dispatcher.getCompletionSuggestions(dispatcher.parse(input, new Object())).join().getList().stream()
				.map(Suggestion::getText).toList();
	}

	@Test
	void completesSubcommandsAndStyles() {
		assertEquals(List.of("heads", "menu", "settings"), complete("yakvc "));
		assertEquals(List.of("bobble", "jaw", "nod", "prosody", "wobble"), complete("yakvc heads "));
	}

	@Test
	void runs() throws Exception {
		for (String command : List.of("yakvc", "yakvc settings", "yakvc menu", "yakvc heads jaw", "yakvc heads bobble 2.5")) {
			dispatcher.execute(command, new Object());
		}
		assertEquals(List.of("settings", "settings", "menu", "jaw null", "bobble 2.5"), ran);
	}
}
