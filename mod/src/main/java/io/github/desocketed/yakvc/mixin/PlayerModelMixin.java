package io.github.desocketed.yakvc.mixin;

import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.HeadMotion;
import io.github.desocketed.yakvc.YakVcClient;
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
		ModelPart head = ((HumanoidModel<?>) (Object) this).head;
		head.xRot += motion.xRot();
		head.yRot += motion.yRot();
		head.zRot += motion.zRot();
	}
}
