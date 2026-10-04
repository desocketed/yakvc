package io.github.desocketed.yakvc.gametest;

import io.github.desocketed.yakvc.YakVcClient;
import net.fabricmc.fabric.api.client.gametest.v1.FabricClientGameTest;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestSingleplayerContext;
import net.fabricmc.loader.api.FabricLoader;

/** Starts the real client, checks the mod and its native engine loaded, and opens a world. */
@SuppressWarnings("UnstableApiUsage")
public class YakVcClientGameTest implements FabricClientGameTest {
	@Override
	public void runTest(ClientGameTestContext context) {
		if (!FabricLoader.getInstance().isModLoaded("yakvc")) {
			throw new AssertionError("yakvc is not loaded");
		}
		if (!YakVcClient.engineLoaded()) {
			throw new AssertionError("yakvc native engine did not load");
		}
		context.takeScreenshot("yakvc-title-screen");

		try (TestSingleplayerContext singleplayer = context.worldBuilder().create()) {
			singleplayer.getConnection().waitForChunksRender();
			context.takeScreenshot("yakvc-singleplayer");
		}
	}
}
