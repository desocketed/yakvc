package io.github.desocketed.yakvc.gametest;

import com.mojang.authlib.GameProfile;
import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.YakVcClient;
import io.github.desocketed.yakvc.natives.NativeBridge;
import java.util.EnumSet;
import java.util.List;
import java.util.UUID;
import net.fabricmc.fabric.api.client.gametest.v1.FabricClientGameTest;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestSingleplayerContext;
import net.fabricmc.fabric.api.entity.FakePlayer;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.player.RemotePlayer;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket.Action;
import net.minecraft.world.level.GameType;

/**
 * Starts the real client with the native engine and dev test audio (see {@code runClientGameTest} in build.gradle),
 * opens a singleplayer world, and checks what the engine is fed: push-to-talk, and spectators excluded both ways.
 */
@SuppressWarnings("UnstableApiUsage")
public class YakVcClientGameTest implements FabricClientGameTest {
	private static final UUID REMOTE = UUID.fromString("00000000-0000-4000-8000-00000000beef");

	@Override
	public void runTest(ClientGameTestContext context) {
		if (!FabricLoader.getInstance().isModLoaded("yakvc")) {
			throw new AssertionError("yakvc is not loaded");
		}
		GameStateFeeder feeder = YakVcClient.feeder();
		if (feeder == null) {
			throw new AssertionError("yakvc native engine did not start");
		}
		context.takeScreenshot("yakvc-title-screen");

		try (TestSingleplayerContext singleplayer = context.worldBuilder().create()) {
			singleplayer.getConnection().waitForChunksRender();
			context.waitFor(mc -> feeder.session() != null && feeder.session().active());
			UUID me = context.computeOnClient(mc -> mc.player.getUUID());

			// Push-to-talk sends the test tone, so the engine reports the local player talking.
			KeyMapping pushToTalk = KeyMapping.get("key.yakvc.push_to_talk");
			context.getInput().holdKey(pushToTalk);
			context.waitFor(mc -> feeder.talking().contains(me));
			context.takeScreenshot("yakvc-talking");
			context.getInput().releaseKey(pushToTalk);
			context.waitFor(mc -> !feeder.talking().contains(me));

			// A remote player is tracked until the tab list says it is a spectator.
			FakePlayer remote = addRemotePlayer(context, singleplayer);
			context.waitFor(mc -> feeder.tracked().contains(REMOTE));
			singleplayer.getServer().runOnServer(server -> {
				remote.setGameMode(GameType.SPECTATOR);
				server.getPlayerList().broadcastAll(new ClientboundPlayerInfoUpdatePacket(Action.UPDATE_GAME_MODE, remote));
			});
			context.waitFor(mc -> !feeder.tracked().contains(REMOTE));

			// A local spectator is flagged, so the engine neither sends nor plays, even with push-to-talk held.
			singleplayer.getServer().runCommand("gamemode spectator @a");
			context.waitFor(mc -> (feeder.inputFlags() & NativeBridge.INPUT_SPECTATOR) != 0);
			context.getInput().holdKey(pushToTalk);
			context.waitTicks(20);
			if (feeder.talking().contains(me)) {
				throw new AssertionError("a spectator transmitted");
			}
			context.getInput().releaseKey(pushToTalk);
			singleplayer.getServer().runCommand("gamemode creative @a");
			context.waitFor(mc -> (feeder.inputFlags() & NativeBridge.INPUT_SPECTATOR) == 0);

			if (feeder.errorEvents() != 0 || feeder.closed()) {
				throw new AssertionError("engine reported errors; see the log");
			}
		}
	}

	/**
	 * Adds a second player: a server-side fake that only exists in the tab list, and a client-side entity two blocks
	 * away, which is all the feeder looks at.
	 */
	private static FakePlayer addRemotePlayer(ClientGameTestContext context, TestSingleplayerContext singleplayer) {
		GameProfile profile = new GameProfile(REMOTE, "Remote");
		FakePlayer remote = singleplayer.getServer().computeOnServer(server -> {
			FakePlayer fake = FakePlayer.get(server.overworld(), profile);
			fake.setGameMode(GameType.SURVIVAL);
			server.getPlayerList().broadcastAll(new ClientboundPlayerInfoUpdatePacket(
					EnumSet.of(Action.ADD_PLAYER, Action.UPDATE_GAME_MODE, Action.UPDATE_LISTED), List.of(fake)));
			return fake;
		});
		context.runOnClient(mc -> {
			RemotePlayer entity = new RemotePlayer(mc.level, profile);
			// Normally the server assigns entity IDs; this one is far above anything a fresh world uses.
			entity.setId(Integer.MAX_VALUE - 1);
			entity.snapTo(mc.player.position().add(2, 0, 0));
			mc.level.addEntity(entity);
		});
		return remote;
	}
}
