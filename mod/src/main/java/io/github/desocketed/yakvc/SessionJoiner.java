package io.github.desocketed.yakvc;

import com.mojang.authlib.exceptions.AuthenticationException;
import java.time.Duration;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.Executor;

/**
 * Proves the player's account to the rendezvous. The rendezvous sends a challenge, the engine turns it into a server
 * id and asks for a {@code JoinRequest}, and this calls Mojang's {@code joinServer} with it so the rendezvous can
 * confirm the join with {@code hasJoined}. The access token stays inside authlib.
 *
 * <p>Never while the game is logging in to a server: Mojang keeps one pending join per account, so ours would replace
 * the game's and the server would kick the player. A join already in flight when the login starts is waited for, so
 * it lands before the game's own.
 *
 * <p>Without an account there is nothing to call: the answer is a failure straight away, and on an offline-mode server
 * the engine then asks for an unverified ticket instead.
 */
public final class SessionJoiner {
	/** authlib's {@code joinServer} for the signed-in account. */
	@FunctionalInterface
	public interface Mojang {
		void joinServer(String serverId) throws AuthenticationException;

		/** False when the game plainly has no Mojang account (see {@link #isAccount}). */
		default boolean hasAccount() {
			return true;
		}
	}

	/**
	 * How long a login waits for a voice join in flight. authlib's HTTP timeouts are longer, but connecting shouldn't
	 * hang on voice; past this the race is back, which is no worse than without the wait.
	 */
	private static final Duration LOGIN_WAIT = Duration.ofSeconds(5);

	/** {@code joinServer} blocks on HTTP, so it runs off the client thread. Requests are rare: one thread each. */
	private static final Executor WORKER = task -> Thread.ofVirtual().name("yakvc-session-join").start(task);

	private final Mojang mojang;
	private final Duration loginWait;
	// Both guarded by this. A join checks loggingIn and marks itself in flight in one step, so a starting login either
	// sees the join in flight and waits for it, or keeps it from starting.
	private boolean loggingIn;
	private boolean joinInFlight;

	public SessionJoiner(Mojang mojang) {
		this(mojang, LOGIN_WAIT);
	}

	SessionJoiner(Mojang mojang, Duration loginWait) {
		this.mojang = mojang;
		this.loginWait = loginWait;
	}

	/**
	 * The game started logging in to a server. Blocks until a voice join in flight finishes, for at most a few
	 * seconds. Fabric's login {@code INIT} fires while the login listener is created, before the game sends its hello,
	 * so the game's own {@code joinServer} only comes after this returns. It runs on the connect thread, or on the
	 * client thread for singleplayer, where a wait only lengthens the loading screen.
	 */
	public synchronized void loginStarted() {
		loggingIn = true;
		long deadline = System.nanoTime() + loginWait.toNanos();
		try {
			while (joinInFlight) {
				long left = deadline - System.nanoTime();
				if (left <= 0) {
					YakVcClient.LOGGER.warn("Voice sign-in still running after {}; logging in anyway", loginWait);
					return;
				}
				wait(left / 1_000_000, (int) (left % 1_000_000));
			}
		} catch (InterruptedException e) {
			Thread.currentThread().interrupt();
		}
	}

	public synchronized void loginEnded() {
		loggingIn = false;
	}

	/** Runs {@link #join} on a worker thread. */
	public CompletableFuture<Boolean> joinAsync(String serverId) {
		return CompletableFuture.supplyAsync(() -> join(serverId), WORKER);
	}

	/**
	 * Whether the launcher gave the game something that can be a Mojang account. Offline launchers give players
	 * their offline UUID (version 3, which no account has) and often no access token; offline developer mode never
	 * signs in.
	 */
	public static boolean isAccount(UUID profileId, String accessToken, boolean offlineDeveloperMode) {
		return !offlineDeveloperMode && profileId.version() != 3 && !accessToken.isBlank();
	}

	/** Calls {@code joinServer} unless there is no account or the game is logging in. Returns whether it succeeded. */
	boolean join(String serverId) {
		if (!mojang.hasAccount()) {
			YakVcClient.LOGGER.info("Voice sign-in skipped: no Minecraft account");
			return false;
		}
		synchronized (this) {
			if (loggingIn) {
				YakVcClient.LOGGER.info("Voice sign-in refused: the game is logging in to a server; retrying later");
				return false;
			}
			joinInFlight = true;
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
		} finally {
			synchronized (this) {
				joinInFlight = false;
				notifyAll();
			}
		}
		return false;
	}
}
