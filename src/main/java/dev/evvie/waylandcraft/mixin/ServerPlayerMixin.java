package dev.evvie.waylandcraft.mixin;

import java.util.ArrayList;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import dev.evvie.waylandcraft.utils.IMyServerPlayer;
import net.minecraft.server.level.ServerPlayer;

@Mixin(ServerPlayer.class)
public class ServerPlayerMixin implements IMyServerPlayer {
	
	private int itemGiveCooldown = 0;
	private ArrayList<Long> aliveWindows = new ArrayList<Long>();
	private boolean screenShareGrabAllowed = false;
	
	@Override
	public int getItemGiveCooldown() {
		return itemGiveCooldown;
	}
	
	@Override
	public void setItemGiveCooldown(int cooldown) {
		itemGiveCooldown = cooldown;
	}
	
	@Override
	public ArrayList<Long> getAliveWindows() {
		return aliveWindows;
	}
	
	@Override
	public boolean isScreenShareGrabAllowed() {
		return screenShareGrabAllowed;
	}
	
	@Override
	public void setScreenShareGrabAllowed(boolean allowed) {
		screenShareGrabAllowed = allowed;
	}
	
	@Inject(method = "restoreFrom", at = @At("TAIL"))
	public void myRestoreFrom(ServerPlayer oldPlayer, boolean restoreAll, CallbackInfo info) {
		aliveWindows = ((IMyServerPlayer) oldPlayer).getAliveWindows();
		screenShareGrabAllowed = ((IMyServerPlayer) oldPlayer).isScreenShareGrabAllowed();
	}

	
}
