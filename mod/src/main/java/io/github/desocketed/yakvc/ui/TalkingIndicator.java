package io.github.desocketed.yakvc.ui;

import com.mojang.blaze3d.vertex.PoseStack;
import io.github.desocketed.yakvc.GameStateFeeder;
import java.util.UUID;
import net.fabricmc.fabric.api.client.rendering.v1.level.LevelRenderContext;
import net.minecraft.client.Minecraft;
import net.minecraft.client.player.AbstractClientPlayer;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.MutableComponent;
import net.minecraft.util.LightCoordsUtil;
import net.minecraft.world.entity.EntityAttachment;
import net.minecraft.world.phys.Vec3;
import org.jspecify.annotations.Nullable;

/**
 * A green speaker over the head of every player who is talking, one line above the name tag, with the verified badge
 * next to it for players whose account is verified, and the group icon over every group mate, talking or not. It is drawn the way vanilla draws name tags, through walls
 * included, from a Fabric level render event rather than a mixin into the player renderer.
 */
public final class TalkingIndicator {
	/** One line of name-tag text, upwards. */
	private static final int ABOVE_NAME_TAG = -10;

	private final GameStateFeeder feeder;

	public TalkingIndicator(GameStateFeeder feeder) {
		this.feeder = feeder;
	}

	public void submit(LevelRenderContext context) {
		Minecraft minecraft = Minecraft.getInstance();
		if ((feeder.talking().isEmpty() && feeder.groupMates().isEmpty()) || minecraft.level == null || minecraft.player == null) return;
		CameraRenderState camera = context.levelState().cameraRenderState;
		float partialTick = context.levelState().worldPartialTicks;
		PoseStack poseStack = context.poseStack();
		for (AbstractClientPlayer player : minecraft.level.players()) {
			Component label = label(player.getUUID(), player == minecraft.player);
			if (label == null || player.isInvisibleTo(minecraft.player)) continue;
			// Your own head is only in view in third person.
			if (player == minecraft.getCameraEntity() && minecraft.options.getCameraType().isFirstPerson()) continue;
			Vec3 attachment = player.getAttachments().getNullable(EntityAttachment.NAME_TAG, 0, player.getYRot(partialTick));
			if (attachment == null) continue;
			Vec3 relative = player.getPosition(partialTick).subtract(camera.pos);
			poseStack.pushPose();
			poseStack.translate(relative.x, relative.y, relative.z);
			// The game shows no name tag over your own head.
			boolean own = player == minecraft.player;
			int offset = own ? 0 : ABOVE_NAME_TAG;
			context.submitNodeCollector().submitNameTag(poseStack, attachment, offset, label, true,
					LightCoordsUtil.FULL_BRIGHT, camera);
			poseStack.popPose();
		}
	}

	/** The icons over {@code uuid}'s head, or null for none. */
	private @Nullable Component label(UUID uuid, boolean own) {
		boolean talking = feeder.talking().contains(uuid);
		boolean mate = feeder.groupMates().contains(uuid);
		if (!talking && !mate) return null;
		MutableComponent label = Component.empty();
		if (talking) {
			label.append(Icons.SPEAKER);
			if (own ? feeder.verified() : feeder.peerVerified(uuid)) label.append(" ").append(Icons.VERIFIED);
		}
		if (mate) {
			if (talking) label.append(" ");
			label.append(Icons.GROUP);
		}
		return label;
	}
}
