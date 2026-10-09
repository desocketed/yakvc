package io.github.desocketed.yakvc.gametest;

import io.github.desocketed.yakvc.YakVcClient;
import io.github.desocketed.yakvc.config.ClientConfig;
import io.github.desocketed.yakvc.natives.EngineEvent;
import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.natives.NativeLoader;
import java.io.IOException;
import java.lang.foreign.MemorySegment;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.UUID;
import net.fabricmc.loader.api.FabricLoader;

/**
 * A second voice engine in the test's own process, standing in for the remote player's game: it signs in to the same
 * dev rendezvous under the remote player's identity, so the two really connect and can form a group. Its events are
 * drained on a thread of its own, which answers the sign-in.
 */
final class RemoteVoice implements AutoCloseable {
	final NativeBridge bridge;
	final MemorySegment engine;
	private final Thread poller;
	private volatile boolean closed;

	RemoteVoice(UUID uuid, String name, UUID localPlayer) throws IOException {
		Path dir = FabricLoader.getInstance().getGameDir().resolve("remote-voice");
		Files.createDirectories(dir);
		// The same library the mod loaded, and the same dev rendezvous and test audio; only the key differs.
		bridge = new NativeBridge(NativeLoader.load(YakVcClient.configDir()));
		String config = Files.readString(YakVcClient.configDir().resolve(ClientConfig.FILE_NAME));
		engine = bridge.create(dir.toString(), config);
		bridge.setIdentity(engine, uuid, name);
		bridge.setTabList(engine, List.of(uuid, localPlayer));
		poller = new Thread(this::poll, "yakvc-remote-voice");
		poller.start();
	}

	private void poll() {
		byte[] buf = new byte[65_540];
		while (!closed) {
			int written;
			while ((written = bridge.pollEvents(engine, buf)) > 0) {
				for (EngineEvent event : EngineEvent.decode(buf, written)) {
					// No Mojang account: the dev rendezvous gives a dev ticket anyway.
					if (event instanceof EngineEvent.JoinRequest join) bridge.completeJoin(engine, join.id(), false);
				}
			}
			try {
				Thread.sleep(50);
			} catch (InterruptedException e) {
				return;
			}
		}
	}

	@Override
	public void close() {
		closed = true;
		try {
			poller.join();
		} catch (InterruptedException e) {
			Thread.currentThread().interrupt();
		}
		bridge.destroy(engine);
	}
}
