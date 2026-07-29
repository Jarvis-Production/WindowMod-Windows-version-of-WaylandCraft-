package dev.evvie.waylandcraft.item;

import java.util.List;
import java.util.UUID;
import java.util.stream.StreamSupport;

import dev.evvie.waylandcraft.utils.IMyServerPlayer;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerTickEvents;
import net.minecraft.core.particles.ParticleTypes;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;

public class ServerItemManager implements ServerTickEvents.StartLevelTick {
	
	@Override
	public void onStartTick(ServerLevel level) {
		for(ServerPlayer player : level.players()) {
			Inventory inv = player.getInventory();
			for(int i = 0; i < inv.getContainerSize(); i++) {
				ItemStack item = inv.getItem(i);
				if(!item.is(WindowItem.WINDOW)) continue;
				
				WindowHandle handle = item.get(WindowItem.WINDOW_HANDLE);
				if(!isHandleValid(level, handle)) {
					inv.setItem(i, ItemStack.EMPTY);
				}
			}
		}
		
		for(ServerPlayer player : level.players()) {
			IMyServerPlayer plr = (IMyServerPlayer) player;
			int itemGiveCooldown = plr.getItemGiveCooldown();
			if(itemGiveCooldown > 0) {
				plr.setItemGiveCooldown(itemGiveCooldown - 1);
			}
		}
		
		StreamSupport.stream(level.getAllEntities().spliterator(), false)
			.filter((e) -> e instanceof ItemEntity)
			.map((e) -> (ItemEntity) e)
			.filter((e) -> e.getItem().is(WindowItem.WINDOW))
			.filter((e) -> !isHandleValid(level, e.getItem().get(WindowItem.WINDOW_HANDLE)))
			.filter((e) -> e.getAge() > 10)
			.forEach((e) -> {
				level.sendParticles(ParticleTypes.FLAME, false, false, e.getX(), e.getY(), e.getZ(), 10, 0.15, 0.2, 0.15, 0.1);
				e.discard();
			});
	}
	
	private static ServerPlayer getPlayer(ServerLevel level, UUID id) {
		for(ServerPlayer player : level.players()) {
			UUID pid = WindowHandle.getPlayerUUID(player);
			if(pid.equals(id)) return player;
		}
		return null;
	}
	
	private boolean isHandleValid(ServerLevel level, WindowHandle handle) {
		if(handle == null) return false;
		
		ServerPlayer player = getPlayer(level, handle.player());
		if(player == null) return false;
		
		return ((IMyServerPlayer) player).getAliveWindows().contains(handle.handle());
	}
	
	public void giveItems(ServerPlayer player, List<Long> handles) {
		for(Long handle : handles) giveItem(player, handle);
	}
	
	public void giveItemsIfMissing(ServerPlayer player, List<Long> handles) {
		for(Long handle : handles) giveItemIfMissing(player, handle);
	}
	
	public void giveItem(ServerPlayer player, long handle) {
		ItemStack item = createItem(player, handle);
		player.addItem(item);
	}
	
	public void giveItemIfMissing(ServerPlayer player, long handle) {
		WindowHandle searched = WindowHandle.forPlayer(player, handle);
		
		// A window item for this handle must NOT be re-created if the player
		// already holds it ANYWHERE in the world — not just in their inventory.
		//
		// Previously we only scanned the player's inventory. When the player
		// THREW the item out, it became an ItemEntity lying on the ground (no
		// longer in the inventory), so the next tick's giveItemsIfMissing found
		// it "missing" and spawned a fresh copy straight back into the inventory
		// — making dropped window items instantly reappear and impossible to get
		// rid of. We now also count the item if it exists as a dropped
		// ItemEntity belonging to this player, so a thrown window stays thrown.
		if(itemExistsForPlayer(player, searched)) return;
		
		ItemStack item = createItem(player, handle);
		player.addItem(item);
	}
	
	/// Does a window item matching `searched` already exist for this player,
	/// either in their inventory or as a dropped ItemEntity in the world?
	private boolean itemExistsForPlayer(ServerPlayer player, WindowHandle searched) {
		Inventory inv = player.getInventory();
		for(int i = 0; i < inv.getContainerSize(); i++) {
			ItemStack item = inv.getItem(i);
			if(!item.is(WindowItem.WINDOW)) continue;
			WindowHandle data = item.get(WindowItem.WINDOW_HANDLE);
			if(data != null && data.equals(searched)) return true;
		}
		
		// Scan dropped ItemEntities in the world for the same window handle so a
		// thrown-out window is not re-handed to the player.
		ServerLevel level = (ServerLevel) player.level();
		return StreamSupport.stream(level.getAllEntities().spliterator(), false)

			.filter((e) -> e instanceof ItemEntity)
			.map((e) -> ((ItemEntity) e).getItem())
			.filter((stack) -> stack.is(WindowItem.WINDOW))
			.map((stack) -> stack.get(WindowItem.WINDOW_HANDLE))
			.anyMatch((data) -> data != null && data.equals(searched));
	}

	
	public static ItemStack createItem(ServerPlayer player, long handle) {
		ItemStack stack = new ItemStack(WindowItem.WINDOW, 1);
		stack.set(WindowItem.WINDOW_HANDLE, WindowHandle.forPlayer(player, handle));
		return stack;
	}
	
}
