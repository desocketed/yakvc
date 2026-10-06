package io.github.desocketed.yakvc;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.mojang.authlib.exceptions.AuthenticationUnavailableException;
import com.mojang.authlib.exceptions.InvalidCredentialsException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.UUID;
import org.junit.jupiter.api.Test;

class SessionJoinerTest {
	@Test
	void joinsWithTheServerId() throws Exception {
		List<String> calls = new ArrayList<>();
		SessionJoiner joiner = new SessionJoiner(calls::add);
		assertTrue(joiner.join("-5a1f"));
		assertTrue(joiner.joinAsync("7b").get());
		assertEquals(List.of("-5a1f", "7b"), calls);
	}

	@Test
	void refusesWhileTheGameIsLoggingIn() {
		List<String> calls = new ArrayList<>();
		SessionJoiner joiner = new SessionJoiner(calls::add);
		joiner.loginStarted();
		assertFalse(joiner.join("1"));
		assertEquals(List.of(), calls, "joinServer must not be called");

		joiner.loginEnded();
		assertTrue(joiner.join("2"));
		assertEquals(List.of("2"), calls);
	}

	@Test
	void withoutAnAccountMojangIsNotCalled() {
		List<String> calls = new ArrayList<>();
		SessionJoiner joiner = new SessionJoiner(new SessionJoiner.Mojang() {
			@Override
			public void joinServer(String serverId) {
				calls.add(serverId);
			}

			@Override
			public boolean hasAccount() {
				return false;
			}
		});
		assertFalse(joiner.join("1"));
		assertEquals(List.of(), calls);
	}

	@Test
	void offlineLaunchersHaveNoAccount() {
		UUID account = UUID.fromString("069a79f4-44e9-4726-a5be-fca90e38aaf5");
		UUID offline = UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(StandardCharsets.UTF_8));
		assertTrue(SessionJoiner.isAccount(account, "token", false));
		assertFalse(SessionJoiner.isAccount(offline, "token", false));
		assertFalse(SessionJoiner.isAccount(account, "", false));
		assertFalse(SessionJoiner.isAccount(account, "token", true));
	}

	@Test
	void mojangErrorsMeanFailure() throws Exception {
		SessionJoiner unreachable = new SessionJoiner(serverId -> {
			throw new AuthenticationUnavailableException("Cannot contact authentication server");
		});
		assertFalse(unreachable.join("1"));

		SessionJoiner badToken = new SessionJoiner(serverId -> {
			throw new InvalidCredentialsException("Invalid token.");
		});
		assertFalse(badToken.joinAsync("1").get());

		SessionJoiner broken = new SessionJoiner(serverId -> {
			throw new IllegalStateException("bug");
		});
		assertFalse(broken.joinAsync("1").get());
	}
}
