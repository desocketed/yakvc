package io.github.desocketed.yakvc.gametest;

import com.mojang.authlib.GameProfile;
import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.YakVcClient;
import io.github.desocketed.yakvc.config.ClientConfig;
import io.github.desocketed.yakvc.config.PlayerVolumes;
import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.ui.VoiceMenuScreen;
import io.github.desocketed.yakvc.ui.VoiceSettingsScreen;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.EnumSet;
import java.util.List;
import java.util.UUID;
import net.fabricmc.fabric.api.client.gametest.v1.FabricClientGameTest;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestSingleplayerContext;
import net.fabricmc.fabric.api.entity.FakePlayer;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.CameraType;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.events.ContainerEventHandler;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;
import com.mojang.blaze3d.platform.InputConstants;
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
			// In third person the speaker shows over the player's own head.
			context.runOnClient(mc -> mc.options.setCameraType(CameraType.THIRD_PERSON_FRONT));
			context.waitTicks(2);
			context.takeScreenshot("yakvc-talking-indicator");
			context.runOnClient(mc -> mc.options.setCameraType(CameraType.FIRST_PERSON));
			context.getInput().releaseKey(pushToTalk);
			context.waitFor(mc -> !feeder.talking().contains(me));

			// A remote player is tracked until the tab list says it is a spectator.
			FakePlayer remote = addRemotePlayer(context, singleplayer);
			context.waitFor(mc -> feeder.tracked().contains(REMOTE));
			checkVoiceMenu(context, feeder);
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
	 * Opens the voice menu, mutes the remote player there, and goes to the settings screen and back. The mute is saved
	 * by UUID; closing the settings unchanged leaves {@code client.toml} alone.
	 */
	private static void checkVoiceMenu(ClientGameTestContext context, GameStateFeeder feeder) {
		Path configFile = YakVcClient.configDir().resolve(ClientConfig.FILE_NAME);
		String configBefore = read(configFile);
		context.setScreen(() -> new VoiceMenuScreen(null, feeder));
		pressNestedButton(context, "yakvc.menu.mute");
		context.waitFor(mc -> feeder.volumes().get(REMOTE).muted());
		context.takeScreenshot("yakvc-voice-menu");

		context.clickScreenButton("yakvc.menu.settings");
		context.waitForScreen(VoiceSettingsScreen.class);
		context.takeScreenshot("yakvc-voice-settings");
		// The privacy settings are below the fold at the test's small window size.
		context.getInput().setCursorPos(context.computeOnClient(mc -> mc.getWindow().getScreenWidth() / 2.0),
				context.computeOnClient(mc -> mc.getWindow().getScreenHeight() / 2.0));
		context.getInput().scroll(-10);
		context.takeScreenshot("yakvc-voice-settings-privacy");
		context.clickScreenButton("gui.done");
		context.waitForScreen(VoiceMenuScreen.class);
		context.clickScreenButton("gui.done");
		context.waitForScreen(null);

		if (!read(YakVcClient.configDir().resolve(PlayerVolumes.FILE_NAME)).contains(REMOTE.toString())) {
			throw new AssertionError("the mute was not saved");
		}
		if (!read(configFile).equals(configBefore)) {
			throw new AssertionError("closing the settings unchanged rewrote client.toml");
		}
		// The run directory is reused, so leave no mute behind for the next run.
		context.runOnClient(mc -> feeder.volumes().set(REMOTE, PlayerVolumes.DEFAULT));
		try {
			feeder.volumes().save();
		} catch (IOException e) {
			throw new UncheckedIOException(e);
		}
	}

	/** Like {@code clickScreenButton}, which only finds buttons placed directly on the screen, but also in lists. */
	private static void pressNestedButton(ClientGameTestContext context, String translationKey) {
		context.runOnClient(mc -> {
			Button button = findButton(mc.gui.screen().children(), Component.translatable(translationKey).getString());
			if (button == null) throw new AssertionError("no button " + translationKey + " in " + mc.gui.screen());
			button.onPress(new KeyEvent(InputConstants.KEY_RETURN, 0, 0));
		});
	}

	private static @Nullable Button findButton(List<? extends GuiEventListener> widgets, String label) {
		for (GuiEventListener widget : widgets) {
			if (widget instanceof Button button && button.getMessage().getString().equals(label)) return button;
			if (widget instanceof ContainerEventHandler container) {
				Button found = findButton(container.children(), label);
				if (found != null) return found;
			}
		}
		return null;
	}

	private static String read(Path file) {
		try {
			return Files.readString(file);
		} catch (IOException e) {
			throw new UncheckedIOException(e);
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
