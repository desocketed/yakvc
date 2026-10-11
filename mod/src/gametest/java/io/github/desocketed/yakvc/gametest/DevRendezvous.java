package io.github.desocketed.yakvc.gametest;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.net.DatagramSocket;
import java.net.InetAddress;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import net.fabricmc.loader.api.FabricLoader;
import net.fabricmc.loader.api.entrypoint.PreLaunchEntrypoint;

/**
 * Starts a dev-mode {@code yakvc-server} for the client gametest and points the mod's {@code client.toml} at it, so
 * voice really connects and the user guide's screenshots show it working. It runs before the mod creates its engine,
 * which reads the config once. The server binary is the {@code yakvc.gametest.server} system property (see
 * build.gradle) and stops with the game.
 */
public class DevRendezvous implements PreLaunchEntrypoint {
	/** Generous: the server is a release binary that only has to bind a socket. */
	private static final Duration START_TIMEOUT = Duration.ofSeconds(60);

	@Override
	public void onPreLaunch() {
		try {
			start();
		} catch (IOException e) {
			throw new UncheckedIOException(e);
		}
	}

	private static void start() throws IOException {
		String server = System.getProperty("yakvc.gametest.server");
		if (server == null) throw new IllegalStateException("-Dyakvc.gametest.server must name a yakvc-server binary");
		Path dir = FabricLoader.getInstance().getGameDir().resolve("rendezvous");
		Files.createDirectories(dir);
		Path endpointKey = dir.resolve("endpoint.key");
		Path issuerKey = dir.resolve("issuer.key");
		// keygen never overwrites a key.
		Files.deleteIfExists(endpointKey);
		Files.deleteIfExists(issuerKey);
		run(server, "keygen", "--out", endpointKey.toString());
		run(server, "keygen", "--issuer", "--out", issuerKey.toString());

		int port = freeUdpPort();
		Path config = dir.resolve("config.toml");
		Files.writeString(config, """
				endpoint_key = "%s"
				issuer_key = "%s"
				bind = "127.0.0.1:%d"
				insecure_dev_auth = true
				""".formatted(endpointKey, issuerKey, port));
		Process process = new ProcessBuilder(server, "run", "--config", config.toString())
				.redirectError(dir.resolve("server.log").toFile())
				.start();
		Runtime.getRuntime().addShutdownHook(new Thread(process::destroy));
		// The server prints its two IDs once it is listening. Read with a deadline: a server that hangs before that
		// would otherwise hang the whole gametest without a word.
		BufferedReader out = process.inputReader();
		String endpointId = value(readLine(out, dir), "endpoint id:", dir);
		String issuerId = value(readLine(out, dir), "issuer id:", dir);

		// Dev test audio stands in for the sound card the gametest client doesn't have, so push-to-talk produces
		// real voice frames and the engine reports no device errors.
		Path clientConfig = FabricLoader.getInstance().getConfigDir().resolve("yakvc/client.toml");
		Files.createDirectories(clientConfig.getParent());
		Files.writeString(clientConfig, """
				dev_mode = true
				trusted_issuers = ["%s"]

				[rendezvous]
				endpoint_id = "%s"
				addrs = ["127.0.0.1:%d"]

				[dev]
				tone_hz = 440.0
				null_output = true
				""".formatted(issuerId, endpointId, port));
		// A fixed client key, so the endpoint id in the debug overlay's screenshot is the same on every run. The engine
		// otherwise makes a random one wherever the run directory is new.
		byte[] nodeKey = new byte[32];
		for (int i = 0; i < nodeKey.length; i++) nodeKey[i] = (byte) (i + 1);
		Files.write(clientConfig.resolveSibling("node.key"), nodeKey);
	}

	private static void run(String... command) throws IOException {
		try {
			int status = new ProcessBuilder(command).inheritIO().start().waitFor();
			if (status != 0) throw new IOException(String.join(" ", command) + " exited with " + status);
		} catch (InterruptedException e) {
			throw new IOException(e);
		}
	}

	/** The next line of the server's output, or an error naming its log when none comes within the timeout. */
	private static String readLine(BufferedReader out, Path dir) throws IOException {
		try {
			return CompletableFuture.supplyAsync(() -> {
				try {
					return out.readLine();
				} catch (IOException e) {
					throw new UncheckedIOException(e);
				}
			}).get(START_TIMEOUT.toMillis(), TimeUnit.MILLISECONDS);
		} catch (TimeoutException e) {
			throw new IOException("yakvc-server printed nothing within " + START_TIMEOUT + "; see " + dir.resolve("server.log"));
		} catch (ExecutionException e) {
			throw new IOException("reading yakvc-server's output failed", e.getCause());
		} catch (InterruptedException e) {
			Thread.currentThread().interrupt();
			throw new IOException(e);
		}
	}

	/** What follows {@code label} on a line of the server's output. */
	private static String value(String line, String label, Path dir) throws IOException {
		if (line == null || !line.startsWith(label)) {
			throw new IOException("yakvc-server did not start; see " + dir.resolve("server.log"));
		}
		return line.substring(label.length()).strip();
	}

	private static int freeUdpPort() throws IOException {
		try (DatagramSocket socket = new DatagramSocket(0, InetAddress.getLoopbackAddress())) {
			return socket.getLocalPort();
		}
	}
}
