package io.github.desocketed.yakvc.gametest;

import com.mojang.authlib.GameProfile;
import com.terraformersmc.modmenu.gui.ModsScreen;
import com.terraformersmc.modmenu.gui.widget.ModListWidget;
import com.terraformersmc.modmenu.gui.widget.entries.ModListEntry;
import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.YakVcClient;
import io.github.desocketed.yakvc.config.ClientConfig;
import io.github.desocketed.yakvc.config.PlayerVolumes;
import io.github.desocketed.yakvc.natives.EngineEvent;
import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.natives.VoiceGroup;
import io.github.desocketed.yakvc.natives.VoiceStats;
import io.github.desocketed.yakvc.ui.GroupsScreen;
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
import java.util.function.Predicate;
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
import net.minecraft.client.gui.components.AbstractButton;
import net.minecraft.client.gui.components.events.ContainerEventHandler;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.components.toasts.SystemToast;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.client.gui.screens.options.SoundOptionsScreen;
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
	/** Normally the server assigns entity IDs; the remote player's is far above anything a fresh world uses. */
	private static final int REMOTE_ENTITY_ID = Integer.MAX_VALUE - 1;
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
		checkModMenu(context);
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
			// The test tone is a working microphone, so the HUD shows no "No microphone".
			context.waitFor(mc -> feeder.micWorking());
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
			// The remote player gets a voice engine of its own, so the menu shows a real connection and the two can
			// form a group. It stops before the debug overlay, whose screenshot would change with its round trip time.
			try (RemoteVoice remoteVoice = remoteVoice(me)) {
				context.waitFor(mc -> feeder.peerState(REMOTE) == EngineEvent.PeerState.DIRECT, 30 * 20);
				checkVoiceMenu(context, feeder);
				checkGroups(context, feeder, remoteVoice, me);
			}
			context.waitFor(mc -> feeder.peerState(REMOTE) == null);
			keyBindsScreenshot(context);
			checkDebugOverlay(context, feeder);
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
			// The feeder's state belongs to the client thread, so it is read there, here and below.
			if (context.computeOnClient(mc -> feeder.talking().contains(me))) {
				throw new AssertionError("a spectator transmitted");
			}
			context.getInput().releaseKey(pushToTalk);
			singleplayer.getServer().runCommand("gamemode creative @a");
			context.waitFor(mc -> (feeder.inputFlags() & NativeBridge.INPUT_SPECTATOR) == 0);

			if (context.computeOnClient(mc -> feeder.errorEvents() != 0 || feeder.closed())) {
				throw new AssertionError("engine reported errors; see the log");
			}
			// After the check above, since these errors are made up.
			checkEngineErrorToasts(context, feeder);
		}
	}

	/** Two engine errors in a row both show, one after the other, rather than the second replacing the first. */
	private static void checkEngineErrorToasts(ClientGameTestContext context, GameStateFeeder feeder) {
		context.runOnClient(mc -> {
			mc.gui.toastManager().clear();
			feeder.handle(new EngineEvent.Error("cannot open the microphone: test"));
			feeder.handle(new EngineEvent.Error("cannot open the speakers: test"));
		});
		SystemToast first = context.computeOnClient(mc ->
				mc.gui.toastManager().getToast(SystemToast.class, VoiceToasts.ENGINE_ERROR));
		if (first == null) throw new AssertionError("no engine error toast");
		// The toasts have no getter for their text, so tell them apart by identity: once the first is gone, another
		// must still be there.
		context.runOnClient(mc -> first.forceHide());
		context.waitFor(mc -> {
			SystemToast next = mc.gui.toastManager().getToast(SystemToast.class, VoiceToasts.ENGINE_ERROR);
			if (next == null) throw new AssertionError("the second engine error toast replaced the first");
			return next != first;
		});
		context.runOnClient(mc -> mc.gui.toastManager().clear());
	}

	/**
	 * Turns the debug overlay on as its settings toggle does (its key is unbound by default), waits for the engine's
	 * first snapshot and takes the user guide's screenshot.
	 */
	private static void checkDebugOverlay(ClientGameTestContext context, GameStateFeeder feeder) {
		if (KeyMapping.get("key.yakvc.debug_overlay") == null) throw new AssertionError("no debug overlay key");
		context.runOnClient(mc -> feeder.setDebugOverlay(true));
		context.waitFor(mc -> feeder.stats() != null);
		VoiceStats stats = context.computeOnClient(mc -> feeder.stats());
		if (stats.endpointId().length() != 64) {
			throw new AssertionError("bad stats snapshot " + stats);
		}
		context.waitTicks(2);
		context.takeScreenshot(TestScreenshotOptions.of("debug-overlay").disableCounterPrefix());
		context.runOnClient(mc -> feeder.setDebugOverlay(false));
	}

	/**
	 * The game's Music &amp; Sounds options open the settings from the title screen too, with no world or voice
	 * session, so they must work there. Done goes back to the sound options.
	 */
	private static void checkSettingsWithoutWorld(ClientGameTestContext context, GameStateFeeder feeder) {
		context.setScreen(() -> new SoundOptionsScreen(new TitleScreen(), Minecraft.getInstance().options));
		context.waitForScreen(SoundOptionsScreen.class);
		// The button is last in the list, below the fold at the test's small window size.
		scrollDown(context);
		screenshotScreen(context, "sound-options");
		pressNestedButton(context, "yakvc.sound_options.voice_chat");
		context.waitForScreen(VoiceSettingsScreen.class);
		// The level meter needs the microphone (here the dev test tone) without a world or push-to-talk.
		context.waitFor(mc -> feeder.micLevelDb() > GameStateFeeder.SILENCE_DB);
		context.clickScreenButton("gui.done");
		context.waitForScreen(SoundOptionsScreen.class);
		context.clickScreenButton("gui.done");
		context.waitForScreen(TitleScreen.class);
	}

	/**
	 * Opens the voice menu, mutes the remote player there, and goes to the settings screen and back. The mute is saved
	 * by UUID; closing the settings unchanged leaves {@code client.toml} alone.
	 */
	private static void checkVoiceMenu(ClientGameTestContext context, GameStateFeeder feeder) {
		Path configFile = YakVcClient.configDir().resolve(ClientConfig.FILE_NAME);
		// DevRendezvous writes a fresh config each run, so the first-run card shows; OK saves that it was seen.
		context.setScreen(() -> new VoiceMenuScreen(null, feeder));
		screenshotScreen(context, "voice-menu-setup");
		context.clickScreenButton("yakvc.setup.ok");
		context.waitFor(mc -> feeder.config().setupDone());
		String configBefore = read(configFile);
		if (!configBefore.contains("setup_done = true")) {
			throw new AssertionError("dismissing the setup card did not save setup_done");
		}
		screenshotScreen(context, "voice-menu");
		pressNestedButton(context, "yakvc.menu.mute");
		context.waitFor(mc -> feeder.volumes().get(REMOTE).muted());

		context.clickScreenButton("yakvc.menu.settings");
		context.waitForScreen(VoiceSettingsScreen.class);
		screenshotScreen(context, "voice-settings");
		pressOptionButton(context, "yakvc.settings.input_device");
		screenshotScreen(context, "voice-settings-microphone");
		pressOptionButton(context, "yakvc.settings.input_device");
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
		context.runOnClient(mc -> {
			feeder.volumes().set(REMOTE, PlayerVolumes.DEFAULT);
			try {
				feeder.volumes().save();
			} catch (IOException e) {
				throw new UncheckedIOException(e);
			}
		});
	}

	private static RemoteVoice remoteVoice(UUID me) {
		try {
			return new RemoteVoice(REMOTE, "Remote", me);
		} catch (IOException e) {
			throw new UncheckedIOException(e);
		}
	}

	/**
	 * The remote player invites us and the Group key, looking at them, accepts; they label the group, which the groups
	 * screen shows; then they leave it.
	 */
	private static void checkGroups(ClientGameTestContext context, GameStateFeeder feeder, RemoteVoice remote,
			UUID me) {
		context.runOnClient(mc -> mc.gui.hud.getChat().clearMessages(true));
		// The engine drops invites from muted players, and the voice menu's mute of the remote player is undone only
		// on the feeder's next tick.
		context.waitTicks(2);
		remote.bridge.invite(remote.engine, me);
		context.waitFor(mc -> feeder.groupChat().invitedBy(REMOTE));
		// The remote player stands two blocks east. It turns to face us: its body otherwise drifts toward its head by how
		// long it has existed, which changes the screenshot from run to run.
		context.runOnClient(mc -> {
			mc.player.setYRot(-90);
			mc.player.setXRot(0);
			if (mc.level.getEntity(REMOTE_ENTITY_ID) instanceof RemotePlayer other) {
				other.setYRot(90);
				other.yRotO = 90;
				other.setYHeadRot(90);
				other.yHeadRotO = 90;
				other.setYBodyRot(90);
				other.yBodyRotO = 90;
			}
		});
		context.waitTicks(2);
		// Arms sway with age in ticks.
		context.runOnClient(mc -> {
			mc.player.tickCount = 0;
			mc.level.getEntity(REMOTE_ENTITY_ID).tickCount = 0;
		});
		context.takeScreenshot(TestScreenshotOptions.of("group-invite").disableCounterPrefix());
		context.getInput().pressKey(KeyMapping.get("key.yakvc.group"));
		context.waitFor(mc -> inGroupWith(feeder, REMOTE));

		remote.bridge.setGroupLabel(remote.engine, "Miners");
		context.waitFor(mc -> feeder.myGroup() != null && feeder.myGroup().label().equals("Miners"));
		// Its toast comes as an event, polled on a later tick than the change shows in the list.
		context.waitTicks(5);
		context.runOnClient(mc -> {
			mc.gui.toastManager().clear();
			mc.gui.hud.getChat().clearMessages(true);
		});
		context.setScreen(() -> new GroupsScreen(null, feeder));
		screenshotScreen(context, "voice-groups");
		context.setScreen(() -> null);

		remote.bridge.leaveGroup(remote.engine);
		context.waitFor(mc -> !inGroupWith(feeder, REMOTE));
		// The same for its toast, which is cleared below.
		context.waitTicks(5);
		context.runOnClient(mc -> {
			feeder.leaveGroup();
			mc.player.setYRot(0);
			mc.gui.hud.getChat().clearMessages(true);
			mc.gui.toastManager().clear();
		});
		context.waitFor(mc -> feeder.myGroup() == null);
	}

	private static boolean inGroupWith(GameStateFeeder feeder, UUID uuid) {
		VoiceGroup mine = feeder.myGroup();
		return mine != null && mine.members().contains(uuid);
	}

	/**
	 * Mod Menu (in dev runs only) lists Yak VC with a config button that opens the settings; Done goes back to the
	 * mods list. Clicked through as a player would, from the title screen's Mods button.
	 */
	private static void checkModMenu(ClientGameTestContext context) {
		context.waitForScreen(TitleScreen.class);
		context.clickScreenButton("modmenu.title");
		context.waitForScreen(ModsScreen.class);
		context.runOnClient(mc -> {
			ModListWidget list = null;
			for (GuiEventListener widget : mc.gui.screen().children()) {
				if (widget instanceof ModListWidget modList) list = modList;
			}
			if (list == null) throw new AssertionError("no mod list in " + mc.gui.screen());
			ModListEntry yakvc = list.children().stream().filter(entry -> entry.getMod().getId().equals("yakvc"))
					.findFirst().orElseThrow(() -> new AssertionError("Yak VC is not in Mod Menu's list"));
			list.select(yakvc);
			// As if clicked; the search box would otherwise keep the focus, and its cursor blinks.
			mc.gui.screen().setFocused(list);
		});
		screenshotScreen(context, "mod-menu");
		// The gear by the selected mod's name; Mod Menu shows it only when the modmenu entrypoint gives a screen.
		String configure = Component.translatable("modmenu.configure").getString();
		context.runOnClient(mc -> {
			AbstractButton gear = findButton(mc.gui.screen().children(), message -> message.equals(configure));
			if (gear == null || !gear.visible || !gear.active) {
				throw new AssertionError("Mod Menu shows no config button for Yak VC; check the log for the modmenu "
						+ "entrypoint");
			}
			gear.onPress(new KeyEvent(InputConstants.KEY_RETURN, 0, 0));
		});
		context.waitForScreen(VoiceSettingsScreen.class);
		context.clickScreenButton("gui.done");
		context.waitForScreen(ModsScreen.class);
		context.clickScreenButton("gui.done");
		context.waitForScreen(TitleScreen.class);
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
		String label = Component.translatable(translationKey).getString();
		pressNestedButton(context, translationKey, message -> message.equals(label));
	}

	/** Presses the nested button whose label starts with the caption, as option buttons show "Caption: value". */
	private static void pressOptionButton(ClientGameTestContext context, String translationKey) {
		String caption = Component.translatable(translationKey).getString() + ":";
		pressNestedButton(context, translationKey, message -> message.startsWith(caption));
	}

	private static void pressNestedButton(ClientGameTestContext context, String name, Predicate<String> label) {
		context.runOnClient(mc -> {
			AbstractButton button = findButton(mc.gui.screen().children(), label);
			if (button == null) throw new AssertionError("no button " + name + " in " + mc.gui.screen());
			button.onPress(new KeyEvent(InputConstants.KEY_RETURN, 0, 0));
		});
	}

	private static @Nullable AbstractButton findButton(List<? extends GuiEventListener> widgets,
			Predicate<String> label) {
		for (GuiEventListener widget : widgets) {
			if (widget instanceof AbstractButton button && label.test(button.getMessage().getString())) return button;
			if (widget instanceof ContainerEventHandler container) {
				AbstractButton found = findButton(container.children(), label);
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
			entity.setId(REMOTE_ENTITY_ID);
			entity.snapTo(mc.player.position().add(2, 0, 0));
			mc.level.addEntity(entity);
		});
		return remote;
	}
}
