package dev.evvie.waylandcraft.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Invoker;

import com.mojang.blaze3d.vertex.PoseStack;

import net.minecraft.client.renderer.ItemInHandRenderer;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.world.entity.HumanoidArm;

@Mixin(ItemInHandRenderer.class)
public interface IItemInHandRendererMixin {
	
	@Invoker("renderPlayerArm")
	void invokeRenderPlayerArm(PoseStack poseStack, SubmitNodeCollector collector, int light, float handHeight, float attack, HumanoidArm humanoidArm);
	
}
