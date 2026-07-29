package dev.evvie.waylandcraft.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import com.mojang.blaze3d.platform.FramerateLimitTracker;

@Mixin(FramerateLimitTracker.class)
public class FramerateLimitTrackerMixin {
	
	@Shadow
	private int framerateLimit;
	
	@Inject(method = "getFramerateLimit", at = @At("HEAD"), cancellable = true)
	public void ovverideFramerateLimit(CallbackInfoReturnable<Integer> info) {
		info.setReturnValue(framerateLimit);
		info.cancel();
	}
	
}
