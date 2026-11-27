package io.github.desocketed.yakvc.gametest;

import com.mojang.authlib.GameProfile;
import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.Occlusion;
import io.github.desocketed.yakvc.Room;
import io.github.desocketed.yakvc.SoundPath;
import io.github.desocketed.yakvc.YakVcClient;
import io.github.desocketed.yakvc.config.SoundTuning;
import io.github.desocketed.yakvc.natives.EngineEvent;
import io.github.desocketed.yakvc.natives.NativeBridge;
import io.github.desocketed.yakvc.natives.VoiceGroup;
import io.github.desocketed.yakvc.natives.VoiceStats;
import io.github.desocketed.yakvc.ui.VoiceToasts;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.util.EnumSet;
import java.util.List;
import java.util.UUID;
import net.fabricmc.fabric.api.client.gametest.v1.FabricClientGameTest;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestSingleplayerContext;
import net.fabricmc.fabric.api.entity.FakePlayer;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.gui.components.toasts.SystemToast;
import net.minecraft.client.player.RemotePlayer;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket.Action;
import net.minecraft.world.level.GameType;
import net.minecraft.world.phys.Vec3;

/**
 * Starts the real client with the native engine, dev test audio and a dev rendezvous ({@link DevRendezvous}), opens a
 * singleplayer world, and checks what the engine is fed: push-to-talk, spectators excluded both ways, groups and the
 * acoustics the world gives.
 */
@SuppressWarnings("UnstableApiUsage")
public class YakVcClientGameTest implements FabricClientGameTest {
	private static final UUID REMOTE = UUID.fromString("00000000-0000-4000-8000-00000000beef");
	/** Normally the server assigns entity IDs; the remote player's is far above anything a fresh world uses. */
	private static final int REMOTE_ENTITY_ID = Integer.MAX_VALUE - 1;

	@Override
	public void runTest(ClientGameTestContext context) {
		if (!FabricLoader.getInstance().isModLoaded("yakvc")) {
			throw new AssertionError("yakvc is not loaded");
		}
		GameStateFeeder feeder = YakVcClient.feeder();
		if (feeder == null) {
			throw new AssertionError("yakvc native engine did not start");
		}

		try (TestSingleplayerContext singleplayer = context.worldBuilder().create()) {
			singleplayer.getServer().runCommand("time set noon");
			singleplayer.getServer().runCommand("tp @a 0.5 -60 0.5 0 0");
			singleplayer.getConnection().waitForChunksRender();
			context.waitFor(mc -> feeder.session() != null && feeder.session().active());
			context.waitFor(mc -> feeder.rendezvous() == EngineEvent.RendezvousState.REGISTERED);
			// Dev tickets count as verified.
			if (!context.computeOnClient(mc -> feeder.verified())) {
				throw new AssertionError("the dev rendezvous gave an unverified ticket");
			}
			UUID me = context.computeOnClient(mc -> mc.player.getUUID());
			// The test tone is a working microphone.
			context.waitFor(mc -> feeder.micWorking());

			// Push-to-talk sends the test tone, so the engine reports the local player talking.
			KeyMapping pushToTalk = KeyMapping.get("key.yakvc.push_to_talk");
			context.getInput().holdKey(pushToTalk);
			context.waitFor(mc -> feeder.talking().contains(me));
			context.getInput().releaseKey(pushToTalk);
			context.waitFor(mc -> !feeder.talking().contains(me));

			// A remote player is tracked until the tab list says it is a spectator.
			FakePlayer remote = addRemotePlayer(context, singleplayer);
			context.waitFor(mc -> feeder.tracked().contains(REMOTE));
			// The remote player gets a voice engine of its own, so the two can connect and form a group.
			try (RemoteVoice remoteVoice = remoteVoice(me)) {
				context.waitFor(mc -> feeder.peerState(REMOTE) == EngineEvent.PeerState.DIRECT, 30 * 20);
				checkGroups(context, feeder, remoteVoice, me);
			}
			context.waitFor(mc -> feeder.peerState(REMOTE) == null);
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

			checkOcclusion(context, singleplayer);
			checkRoom(context, singleplayer);
			checkSoundPath(context, singleplayer);
			checkFullSendsTheRoom(context, singleplayer, feeder);

			if (context.computeOnClient(mc -> feeder.errorEvents() != 0 || feeder.closed())) {
				throw new AssertionError("engine reported errors; see the log");
			}
			// After the check above, since these errors are made up.
			checkEngineErrorToasts(context, feeder);
		}
	}

	/** Blocks between two points muffle by what they are made of; the ray must actually cross a block's shape. */
	private static void checkOcclusion(ClientGameTestContext context, TestSingleplayerContext singleplayer) {
		Vec3 from = new Vec3(12.5, -58.5, 3.5);
		Vec3 to = new Vec3(12.5, -58.5, 7.5);
		String[][] cases = {
				{"air", "0"},
				{"stone", "0.5"},
				{"white_wool", "0.85"},
				{"glass", "0.3"},
				{"oak_fence_gate[facing=north,open=true]", "0"},
				{"oak_fence_gate[facing=north,open=false]", "0.5"},
		};
		for (String[] blockAndExpected : cases) {
			singleplayer.getServer().runCommand("setblock 12 -59 5 " + blockAndExpected[0]);
			context.waitTicks(3);
			float occlusion = context.computeOnClient(mc -> Occlusion.between(mc.level, from, to));
			if (Math.abs(occlusion - Float.parseFloat(blockAndExpected[1])) > 0.001f) {
				throw new AssertionError(blockAndExpected[0] + " occludes " + occlusion + ", expected "
						+ blockAndExpected[1]);
			}
		}
		singleplayer.getServer().runCommand("setblock 12 -59 5 air");
	}
	/** Open ground has almost no reverb; a small stone box has plenty, and a short one; a big hall rings longer. */
	private static void checkRoom(ClientGameTestContext context, TestSingleplayerContext singleplayer) {
		Vec3 eye = new Vec3(20.5, -58.5, 20.5);
		Room open = context.computeOnClient(mc -> Room.around(mc.level, eye, SoundTuning.DEFAULT));
		singleplayer.getServer().runCommand("fill 18 -61 18 22 -56 22 stone hollow");
		context.waitTicks(3);
		Room small = context.computeOnClient(mc -> Room.around(mc.level, eye, SoundTuning.DEFAULT));
		singleplayer.getServer().runCommand("fill 18 -61 18 22 -56 22 air");
		singleplayer.getServer().runCommand("fill 10 -61 10 30 -46 30 stone hollow");
		context.waitTicks(3);
		Room hall = context.computeOnClient(mc -> Room.around(mc.level, eye, SoundTuning.DEFAULT));
		singleplayer.getServer().runCommand("fill 10 -61 10 30 -46 30 air");
		singleplayer.getServer().runCommand("fill 10 -61 10 30 -61 30 grass_block");
		if (!(open.reverb() < 0.1f && small.reverb() > 0.5f && hall.reverb() > 0.5f && hall.decay() > small.decay())) {
			throw new AssertionError("rooms: open " + open + ", small " + small + ", hall " + hall);
		}
	}
	/**
	 * A voice behind a thick wall comes round its open end, from that side and less muffled than through it; shut in
	 * with no way round, it comes straight through.
	 */
	private static void checkSoundPath(ClientGameTestContext context, TestSingleplayerContext singleplayer) {
		Vec3 listener = new Vec3(40.5, -58.5, 40.5);
		Vec3 speaker = new Vec3(40.5, -58.5, 48.5);
		// Too tall to go over within the search box, and the way round is short only past x 44.
		singleplayer.getServer().runCommand("fill 26 -60 43 44 -45 45 stone");
		context.waitTicks(3);
		SoundPath around = context.computeOnClient(mc -> SoundPath.between(mc.level, listener, speaker, SoundTuning.DEFAULT));
		if (!(around.occlusion() > 0.4f && around.occlusion() < 0.875f && around.heardFrom().x > 41)) {
			throw new AssertionError("round the wall: " + around);
		}
		// Wall the end off and cut a doorway beside the players: a closed door is no way round (the voice comes
		// through the wall or the long way round the far end, either much more blocked), an open one is.
		singleplayer.getServer().runCommand("fill 45 -60 43 47 -45 45 stone");
		singleplayer.getServer().runCommand("fill 41 -59 43 41 -58 45 air");
		singleplayer.getServer().runCommand("setblock 41 -59 44 oak_door[facing=north,half=lower]");
		singleplayer.getServer().runCommand("setblock 41 -58 44 oak_door[facing=north,half=upper]");
		context.waitTicks(3);
		SoundPath closed = context.computeOnClient(mc -> SoundPath.between(mc.level, listener, speaker, SoundTuning.DEFAULT));
		singleplayer.getServer().runCommand("setblock 41 -59 44 oak_door[facing=north,half=lower,open=true]");
		singleplayer.getServer().runCommand("setblock 41 -58 44 oak_door[facing=north,half=upper,open=true]");
		context.waitTicks(3);
		SoundPath open = context.computeOnClient(mc -> SoundPath.between(mc.level, listener, speaker, SoundTuning.DEFAULT));
		if (!(closed.occlusion() > open.occlusion() + 0.3f && open.occlusion() < 0.6f && open.heardFrom().x > 40.5)) {
			throw new AssertionError("doors: closed " + closed + ", open " + open);
		}
		singleplayer.getServer().runCommand("fill 20 -60 43 60 -45 45 stone");
		context.waitTicks(3);
		SoundPath through = context.computeOnClient(mc -> SoundPath.between(mc.level, listener, speaker, SoundTuning.DEFAULT));
		singleplayer.getServer().runCommand("fill 20 -60 43 60 -45 45 air");
		if (!(through.occlusion() > 0.85f && through.heardFrom().equals(speaker))) {
			throw new AssertionError("through the wall: " + through);
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
	private static RemoteVoice remoteVoice(UUID me) {
		try {
			return new RemoteVoice(REMOTE, "Remote", me);
		} catch (IOException e) {
			throw new UncheckedIOException(e);
		}
	}
	private static boolean inGroupWith(GameStateFeeder feeder, UUID uuid) {
		VoiceGroup mine = feeder.myGroup();
		return mine != null && mine.members().contains(uuid);
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

	/** The remote player invites us, we accept, they label the group, then they leave it. */
	private static void checkGroups(ClientGameTestContext context, GameStateFeeder feeder, RemoteVoice remote,
			UUID me) {
		remote.bridge.invite(remote.engine, me);
		context.waitFor(mc -> feeder.acceptInvite(REMOTE));
		context.waitFor(mc -> inGroupWith(feeder, REMOTE));

		remote.bridge.setGroupLabel(remote.engine, "Miners");
		context.waitFor(mc -> feeder.myGroup() != null && feeder.myGroup().label().equals("Miners"));

		remote.bridge.leaveGroup(remote.engine);
		context.waitFor(mc -> !inGroupWith(feeder, REMOTE));
		context.runOnClient(mc -> feeder.leaveGroup());
		context.waitFor(mc -> feeder.myGroup() == null);
	}

	/** Waits for the engine's first stats snapshot, which the debug overlay shows. */
	private static void checkDebugOverlay(ClientGameTestContext context, GameStateFeeder feeder) {
		if (KeyMapping.get("key.yakvc.debug_overlay") == null) throw new AssertionError("no debug overlay key");
		context.runOnClient(mc -> feeder.setDebugOverlay(true));
		context.waitFor(mc -> feeder.stats() != null);
		VoiceStats stats = context.computeOnClient(mc -> feeder.stats());
		if (stats.endpointId().length() != 64) {
			throw new AssertionError("bad stats snapshot " + stats);
		}
		context.runOnClient(mc -> feeder.setDebugOverlay(false));
	}

	/** In a stone box around the player, the default Spatial Audio level (Full) sends the box's reverb. */
	private static void checkFullSendsTheRoom(ClientGameTestContext context, TestSingleplayerContext singleplayer,
			GameStateFeeder feeder) {
		singleplayer.getServer().runCommand("fill -2 -61 -2 2 -56 2 stone hollow");
		context.waitFor(mc -> feeder.room().reverb() > 0.5f);
		singleplayer.getServer().runCommand("fill -2 -61 -2 2 -56 2 air");
		singleplayer.getServer().runCommand("fill -2 -61 -2 2 -61 2 grass_block");
	}
}
