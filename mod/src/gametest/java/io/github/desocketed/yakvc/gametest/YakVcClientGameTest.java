package io.github.desocketed.yakvc.gametest;

import com.mojang.authlib.GameProfile;
import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.YakVcClient;
import io.github.desocketed.yakvc.config.ClientConfig;
import io.github.desocketed.yakvc.config.PlayerVolumes;
import io.github.desocketed.yakvc.natives.EngineEvent;
import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.ui.VoiceMenuScreen;
import io.github.desocketed.yakvc.ui.VoiceSettingsScreen;
import io.github.desocketed.yakvc.ui.VoiceToasts;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.image.BufferedImage;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.EnumSet;
import java.util.List;
import java.util.UUID;
import javax.imageio.ImageIO;
import net.fabricmc.fabric.api.client.gametest.v1.FabricClientGameTest;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestSingleplayerContext;
import net.fabricmc.fabric.api.client.gametest.v1.screenshot.TestScreenshotOptions;
import net.fabricmc.fabric.api.entity.FakePlayer;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.CameraType;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.AbstractScrollArea;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.events.ContainerEventHandler;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.client.gui.screens.options.controls.KeyBindsScreen;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;
import com.mojang.blaze3d.platform.InputConstants;
import net.minecraft.client.player.RemotePlayer;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket.Action;
import net.minecraft.world.level.GameType;

/**
 * Starts the real client with the native engine, dev test audio and a dev rendezvous ({@link DevRendezvous}), opens a
 * singleplayer world, and checks what the engine is fed: push-to-talk, and spectators excluded both ways.
 *
 * <p>On the way it takes the screenshots in {@code docs/USER_GUIDE.md}, of every HUD state and screen. They keep
 * their names (no counter prefix), and {@code scripts/UpdateGuideScreenshots.java} copies them into {@code
 * docs/guide/} when they change. The world, time, camera and cursor are fixed, so unchanged UI gives the same images.
 */
@SuppressWarnings("UnstableApiUsage")
public class YakVcClientGameTest implements FabricClientGameTest {
	private static final UUID REMOTE = UUID.fromString("00000000-0000-4000-8000-00000000beef");
	/** The HUD's corner of the 854×480 test window, at its GUI scale of 2. */
	private static final int HUD_WIDTH = 76;
	private static final int HUD_HEIGHT = 26;

	@Override
	public void runTest(ClientGameTestContext context) {
		if (!FabricLoader.getInstance().isModLoaded("yakvc")) {
			throw new AssertionError("yakvc is not loaded");
		}
		GameStateFeeder feeder = YakVcClient.feeder();
		if (feeder == null) {
			throw new AssertionError("yakvc native engine did not start");
		}

		// Both change with timing: the vignette fades with the light, and opening a menu in singleplayer saves the
		// world, which shows "Saving world" for a moment.
		context.runOnClient(mc -> {
			mc.options.vignette().set(false);
			mc.options.showAutosaveIndicator().set(false);
		});
		checkSettingsWithoutWorld(context, feeder);
		try (TestSingleplayerContext singleplayer = context.worldBuilder().create()) {
			// The world builder already fixes the seed and weather and stops time; this fixes the time and view.
			singleplayer.getServer().runCommand("time set noon");
			singleplayer.getServer().runCommand("tp @a 0.5 -60 0.5 0 0");
			singleplayer.getConnection().waitForChunksRender();
			context.waitFor(mc -> feeder.session() != null && feeder.session().active());
			context.waitFor(mc -> feeder.rendezvous() == EngineEvent.RendezvousState.REGISTERED);
			// Dev tickets count as verified, so the badge shows in the talking indicator and the settings.
			if (!context.computeOnClient(mc -> feeder.verified())) {
				throw new AssertionError("the dev rendezvous gave an unverified ticket");
			}
			UUID me = context.computeOnClient(mc -> mc.player.getUUID());
			hudScreenshot(context, "hud-ready");

			// Push-to-talk sends the test tone, so the engine reports the local player talking.
			KeyMapping pushToTalk = KeyMapping.get("key.yakvc.push_to_talk");
			context.getInput().holdKey(pushToTalk);
			context.waitFor(mc -> feeder.talking().contains(me));
			hudScreenshot(context, "hud-talking");
			// In third person the speaker shows over the player's own head. Looking up puts the camera above,
			// looking down at grass only: how far the terrain reaches depends on how many chunks have loaded.
			context.runOnClient(mc -> {
				mc.player.setXRot(-45);
				mc.options.setCameraType(CameraType.THIRD_PERSON_FRONT);
			});
			context.waitTicks(2);
			// The arms sway with the player's age in ticks.
			context.runOnClient(mc -> mc.player.tickCount = 0);
			context.takeScreenshot(TestScreenshotOptions.of("talking-indicator").disableCounterPrefix());
			context.runOnClient(mc -> {
				mc.player.setXRot(0);
				mc.options.setCameraType(CameraType.FIRST_PERSON);
			});
			context.getInput().releaseKey(pushToTalk);
			context.waitFor(mc -> !feeder.talking().contains(me));

			context.runOnClient(mc -> feeder.keys().setMuted(true));
			context.waitTicks(2);
			hudScreenshot(context, "hud-muted");
			context.runOnClient(mc -> {
				feeder.keys().setMuted(false);
				feeder.keys().setDeafened(true);
			});
			context.waitTicks(2);
			hudScreenshot(context, "hud-deafened");
			context.runOnClient(mc -> feeder.keys().setDeafened(false));

			toastScreenshot(context);

			// A remote player is tracked until the tab list says it is a spectator.
			FakePlayer remote = addRemotePlayer(context, singleplayer);
			context.waitFor(mc -> feeder.tracked().contains(REMOTE));
			checkVoiceMenu(context, feeder);
			keyBindsScreenshot(context);
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
	 * Mod Menu opens the settings from the title screen, with no world or voice session, so they must work there too.
	 * Mod Menu isn't on the test classpath, so the screen is opened directly, as its entry does.
	 */
	private static void checkSettingsWithoutWorld(ClientGameTestContext context, GameStateFeeder feeder) {
		context.setScreen(() -> new VoiceSettingsScreen(new TitleScreen(), Minecraft.getInstance().options, feeder));
		context.waitForScreen(VoiceSettingsScreen.class);
		context.clickScreenButton("gui.done");
		context.waitForScreen(TitleScreen.class);
	}

	/**
	 * Opens the voice menu, mutes the remote player there, and goes to the settings screen and back. The mute is saved
	 * by UUID; closing the settings unchanged leaves {@code client.toml} alone.
	 */
	private static void checkVoiceMenu(ClientGameTestContext context, GameStateFeeder feeder) {
		Path configFile = YakVcClient.configDir().resolve(ClientConfig.FILE_NAME);
		String configBefore = read(configFile);
		context.setScreen(() -> new VoiceMenuScreen(null, feeder));
		screenshotScreen(context, "voice-menu");
		pressNestedButton(context, "yakvc.menu.mute");
		context.waitFor(mc -> feeder.volumes().get(REMOTE).muted());

		context.clickScreenButton("yakvc.menu.settings");
		context.waitForScreen(VoiceSettingsScreen.class);
		screenshotScreen(context, "voice-settings");
		// The privacy settings are below the fold at the test's small window size.
		scrollDown(context);
		screenshotScreen(context, "voice-settings-privacy");
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

	/** The game's own key binds screen, scrolled to the Yak VC keys at the end. */
	private static void keyBindsScreenshot(ClientGameTestContext context) {
		context.setScreen(() -> new KeyBindsScreen(null, context.computeOnClient(mc -> mc.options)));
		scrollDown(context);
		screenshotScreen(context, "key-binds");
		context.setScreen(() -> null);
	}

	/** An example failure toast, in the top-right corner. */
	private static void toastScreenshot(ClientGameTestContext context) {
		context.runOnClient(mc -> VoiceToasts.show(Component.translatable("yakvc.toast.crashed")));
		// Toasts slide in by the clock from their first frame, not by ticks.
		context.waitTicks(2);
		try {
			Thread.sleep(1000);
		} catch (InterruptedException e) {
			throw new AssertionError(e);
		}
		context.waitTicks(2);
		Path path = context.takeScreenshot(TestScreenshotOptions.of("toast").disableCounterPrefix());
		cropTop(path, true, 470, 90, 1);
		context.runOnClient(mc -> mc.gui.toastManager().clear());
	}

	/** Scrolls the open screen's list to the end. */
	private static void scrollDown(ClientGameTestContext context) {
		context.runOnClient(mc -> {
			for (GuiEventListener widget : mc.gui.screen().children()) {
				if (widget instanceof AbstractScrollArea list) list.setScrollAmount(list.maxScrollAmount());
			}
		});
	}

	/**
	 * A screenshot of the open screen for the user guide, with the cursor in the corner so nothing shows as hovered.
	 */
	private static void screenshotScreen(ClientGameTestContext context, String name) {
		context.getInput().setCursorPos(0, 0);
		context.waitTicks(2);
		context.takeScreenshot(TestScreenshotOptions.of(name).disableCounterPrefix());
	}

	/** The HUD's icons in the top-left corner, for the user guide, enlarged so the small icons are easy to see. */
	private static void hudScreenshot(ClientGameTestContext context, String name) {
		Path path = context.takeScreenshot(TestScreenshotOptions.of(name).disableCounterPrefix());
		cropTop(path, false, HUD_WIDTH, HUD_HEIGHT, 3);
	}

	/** Keeps only the top-left or top-right corner of a screenshot, enlarged {@code scale} times with sharp pixels. */
	private static void cropTop(Path png, boolean right, int width, int height, int scale) {
		try {
			BufferedImage image = ImageIO.read(png.toFile());
			int x = right ? image.getWidth() - width : 0;
			BufferedImage out = new BufferedImage(width * scale, height * scale, BufferedImage.TYPE_INT_RGB);
			Graphics2D graphics = out.createGraphics();
			graphics.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_NEAREST_NEIGHBOR);
			graphics.drawImage(image.getSubimage(x, 0, width, height), 0, 0, width * scale, height * scale, null);
			graphics.dispose();
			ImageIO.write(out, "png", png.toFile());
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
