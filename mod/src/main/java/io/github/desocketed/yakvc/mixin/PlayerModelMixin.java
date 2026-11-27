package io.github.desocketed.yakvc.mixin;

import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.YakVcClient;
import io.github.desocketed.yakvc.heads.Distortion;
import io.github.desocketed.yakvc.heads.HeadMotion;
import net.minecraft.client.Minecraft;
import net.minecraft.client.model.HumanoidModel;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.renderer.entity.state.AvatarRenderState;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.Pose;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Moves a talking player's head with their voice ({@link HeadMotion}), on top of where they look. Fabric API has no
 * event for model poses, so this is the mod's one mixin.
 */
@Mixin(PlayerModel.class)
abstract class PlayerModelMixin {
	@Inject(method = "setupAnim(Lnet/minecraft/client/renderer/entity/state/AvatarRenderState;)V", at = @At("TAIL"))
	private void yakvc$speakingHead(AvatarRenderState state, CallbackInfo ci) {
		if (state.pose != Pose.STANDING) return;
		GameStateFeeder feeder = YakVcClient.feeder();
		Minecraft minecraft = Minecraft.getInstance();
		if (feeder == null || minecraft.level == null) return;
		Entity entity = minecraft.level.getEntity(state.id);
		if (entity == null) return;
		HeadMotion motion = feeder.heads().at(entity.getUUID(), System.nanoTime());
		if (motion == null) return;
		float strength = (float) feeder.settings().headsStrength();
		ModelPart head = ((HumanoidModel<?>) (Object) this).head;
		head.xRot += strength * motion.xRot();
		head.yRot += strength * motion.yRot();
		head.zRot += strength * motion.zRot();
		Distortion distortion = feeder.heads().distortion(entity.getUUID());
		if (distortion == null) return;
		float distort = (float) feeder.settings().distortStrength();
		// Kept from inverting or exploding the head.
		head.xScale *= Math.clamp(1 + distort * distortion.xScale(), 0.3f, 2f);
		head.yScale *= Math.clamp(1 + distort * distortion.yScale(), 0.3f, 2f);
		head.zScale *= Math.clamp(1 + distort * distortion.zScale(), 0.3f, 2f);
	}
}
