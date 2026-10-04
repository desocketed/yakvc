package io.github.desocketed.yakvc.gametest;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.net.DatagramSocket;
import java.net.InetAddress;
import java.nio.file.Files;
import java.nio.file.Path;
import net.fabricmc.loader.api.FabricLoader;
import net.fabricmc.loader.api.entrypoint.PreLaunchEntrypoint;

/**
 * Starts a dev-mode {@code yakvc-server} for the client gametest and points the mod's {@code client.toml} at it, so
 * voice really connects and the user guide's screenshots show it working. It runs before the mod creates its engine,
 * which reads the config once. The server binary is the {@code yakvc.gametest.server} system property (see
 * build.gradle) and stops with the game.
 */
public class DevRendezvous implements PreLaunchEntrypoint {
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
		// The server prints its two IDs once it is listening.
		BufferedReader out = process.inputReader();
		String endpointId = value(out.readLine(), "endpoint id:", dir);
		String issuerId = value(out.readLine(), "issuer id:", dir);

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
	}

	private static void run(String... command) throws IOException {
		try {
			int status = new ProcessBuilder(command).inheritIO().start().waitFor();
			if (status != 0) throw new IOException(String.join(" ", command) + " exited with " + status);
		} catch (InterruptedException e) {
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
