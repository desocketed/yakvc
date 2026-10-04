package io.github.desocketed.yakvc;

import com.mojang.authlib.exceptions.AuthenticationException;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.Executor;

/**
 * Proves the player's account to the rendezvous. The rendezvous sends a challenge, the engine turns it into a server
 * id and asks for a {@code JoinRequest}, and this calls Mojang's {@code joinServer} with it so the rendezvous can
 * confirm the join with {@code hasJoined}. The access token stays inside authlib.
 *
 * <p>Never while the game is logging in to a server: Mojang keeps one pending join per account, so ours would replace
 * the game's and the server would kick the player.
 */
public final class SessionJoiner {
	/** authlib's {@code joinServer} for the signed-in account. */
	@FunctionalInterface
	public interface Mojang {
		void joinServer(String serverId) throws AuthenticationException;
	}

	/** {@code joinServer} blocks on HTTP, so it runs off the client thread. Requests are rare: one thread each. */
	private static final Executor WORKER = task -> Thread.ofVirtual().name("yakvc-session-join").start(task);

	private final Mojang mojang;
	/** Set from network threads, read on the worker. */
	private volatile boolean loggingIn;

	public SessionJoiner(Mojang mojang) {
		this.mojang = mojang;
	}

	public void loginStarted() {
		loggingIn = true;
	}

	public void loginEnded() {
		loggingIn = false;
	}

	/** Runs {@link #join} on a worker thread. */
	public CompletableFuture<Boolean> joinAsync(String serverId) {
		return CompletableFuture.supplyAsync(() -> join(serverId), WORKER);
	}

	/** Calls {@code joinServer} unless the game is logging in. Returns whether it succeeded. */
	boolean join(String serverId) {
		if (loggingIn) {
			YakVcClient.LOGGER.info("Voice sign-in refused: the game is logging in to a server; retrying later");
			return false;
		}
		try {
			mojang.joinServer(serverId);
			YakVcClient.LOGGER.info("Voice sign-in: joined the Mojang session");
			return true;
		} catch (AuthenticationException e) {
			// Covers an unreachable session server and invalid or expired credentials alike.
			YakVcClient.LOGGER.warn("Voice sign-in failed: {}: {}", e.getClass().getSimpleName(), e.getMessage());
		} catch (RuntimeException e) {
			YakVcClient.LOGGER.error("Voice sign-in failed", e);
		}
		return false;
	}
}
