package io.github.desocketed.yakvc;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.mojang.authlib.exceptions.AuthenticationUnavailableException;
import com.mojang.authlib.exceptions.InvalidCredentialsException;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
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
	void aLoginWaitsForAJoinInFlight() throws Exception {
		CountDownLatch joinStarted = new CountDownLatch(1);
		CountDownLatch releaseJoin = new CountDownLatch(1);
		List<String> order = Collections.synchronizedList(new ArrayList<>());
		SessionJoiner joiner = new SessionJoiner(serverId -> {
			joinStarted.countDown();
			await(releaseJoin);
			order.add("voice join");
		});
		CompletableFuture<Boolean> join = joiner.joinAsync("1");
		assertTrue(joinStarted.await(5, TimeUnit.SECONDS));

		Thread login = Thread.ofVirtual().start(() -> {
			joiner.loginStarted();
			order.add("game login");
		});
		login.join(200);
		assertTrue(login.isAlive(), "the login must wait for the voice join");
		releaseJoin.countDown();
		login.join(5_000);
		assertTrue(join.get());
		assertEquals(List.of("voice join", "game login"), order);
	}

	@Test
	void aLoginWaitsOnlyBriefly() throws Exception {
		CountDownLatch joinStarted = new CountDownLatch(1);
		CountDownLatch releaseJoin = new CountDownLatch(1);
		SessionJoiner joiner = new SessionJoiner(serverId -> {
			joinStarted.countDown();
			await(releaseJoin);
		}, Duration.ofMillis(100));
		CompletableFuture<Boolean> join = joiner.joinAsync("1");
		assertTrue(joinStarted.await(5, TimeUnit.SECONDS));

		long start = System.nanoTime();
		joiner.loginStarted();
		long waited = System.nanoTime() - start;
		assertTrue(waited >= Duration.ofMillis(100).toNanos(), "waited " + waited + " ns");
		assertTrue(waited < Duration.ofSeconds(4).toNanos(), "waited " + waited + " ns");
		releaseJoin.countDown();
		assertTrue(join.get());
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

	/** {@code joinServer} can't throw {@code InterruptedException}. */
	private static void await(CountDownLatch latch) {
		try {
			latch.await();
		} catch (InterruptedException e) {
			throw new IllegalStateException(e);
		}
	}
}
