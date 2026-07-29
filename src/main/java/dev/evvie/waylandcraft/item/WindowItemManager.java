package dev.evvie.waylandcraft.item;

import java.util.Arrays;

import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.WaylandCraftCommon;
import dev.evvie.waylandcraft.bridge.WLCToplevel;
import dev.evvie.waylandcraft.desktop.DesktopEntry;
import dev.evvie.waylandcraft.network.ServerboundAliveWindowsPayload;
import dev.evvie.waylandcraft.network.ServerboundGiveItemsPayload;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.fabricmc.fabric.api.client.networking.v1.ClientPlayNetworking;
import net.minecraft.client.Minecraft;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.item.ItemStack;

public class WindowItemManager implements WindowItemInteractionProvider, ClientTickEvents.StartTick {
	
	private static Component UNKNOWN_WINDOW_TEXT = Component.literal("Unknown Window");
	
	// Toplevel handles last sent to the server for synchronization
	private long[] syncedToplevels = new long[0];
	
	public void giveItem(WLCToplevel toplevel) {
		ClientPlayNetworking.send(new ServerboundGiveItemsPayload(new long[] {toplevel.getHandle()}, false));
	}
	
	public void giveItemsIfMissing(WLCToplevel... toplevels) {
		if(toplevels.length < 1) return;
		
		long[] handles = new long[toplevels.length];
		for(int i = 0; i < toplevels.length; i++) handles[i] = toplevels[i].getHandle();
		
		syncToplevels();
		ClientPlayNetworking.send(new ServerboundGiveItemsPayload(handles, true));
	}
	
	@Override
	public void onStartTick(Minecraft client) {
		if(client.level == null) return;
		if(WaylandCraft.instance.bridge == null) return;
		
		// Keep the server's view of which windows are still alive up to date so
		// it can invalidate items for windows that have closed.
		syncToplevels();
		
		// NOTE: we deliberately do NOT re-request items for ALL current toplevels
		// here every tick. Doing so re-handed the player an item for EVERY window
		// that is still alive, which meant a window-item the player intentionally
		// THREW AWAY (dropped) was immediately recreated on the next tick — the
		// item "restored itself" and could never be discarded while its app was
		// open.
		//
		// Items for windows that have just appeared are still handed out exactly
		// once, in WaylandCraft.updateWorld via giveItemsIfMissing(getNewToplevels()).
		// That covers "newly opened apps appear as an item" without the
		// auto-restore side effect, so a dropped item now stays dropped.
	}


	
	public void syncToplevels() {
		long[] handles = Arrays.stream(WaylandCraft.instance.bridge.getToplevels()).mapToLong((t) -> t.getHandle()).toArray();
		if(Arrays.equals(handles, syncedToplevels)) return;
		
		ClientPlayNetworking.send(new ServerboundAliveWindowsPayload(handles));
		syncedToplevels = handles;
	}
	
	public boolean isValid(ItemStack itemStack) {
		WLCToplevel toplevel = WaylandCraft.getToplevel(itemStack);
		return toplevel != null;
	}
	
	@Override
	public Component getName(ItemStack itemStack) {
		WLCToplevel toplevel = WaylandCraft.getToplevel(itemStack);
		if(toplevel == null) return UNKNOWN_WINDOW_TEXT;
		
		DesktopEntry entry = WaylandCraft.instance.xdgManager.forAppId(toplevel.appID);
		if(entry == null) return UNKNOWN_WINDOW_TEXT;
		
		String name = entry.name;
		if(name == null) return UNKNOWN_WINDOW_TEXT;
		
		return Component.literal(name);
	}
	
	@Override
	public void useTick(LivingEntity entity, ItemStack itemStack) {
		if(entity != Minecraft.getInstance().player) return;
		WaylandCraft.instance.startUsingWindowItem();
	}
	
}
