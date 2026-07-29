package dev.evvie.waylandcraft;

import org.jetbrains.annotations.Nullable;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import dev.evvie.waylandcraft.item.ServerItemManager;
import dev.evvie.waylandcraft.item.WindowItem;
import dev.evvie.waylandcraft.item.WindowItemInteractionProvider;
import dev.evvie.waylandcraft.network.WaylandCraftNetworking;
import net.fabricmc.api.ModInitializer;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerTickEvents;

public class WaylandCraftCommon implements ModInitializer {
	
	public static final String MOD_ID = "waylandcraft";
	public static final Logger LOGGER = LoggerFactory.getLogger(MOD_ID);
	public static WaylandCraftCommon instance;
	
	public @Nullable WindowItemInteractionProvider windowItemInteractionProvider = null;
	public ServerItemManager serverItemManager = new ServerItemManager();
	
	// Server-side switch controlling whether shared screens are relayed to all
	// players. When false the server drops incoming frames, effectively hiding
	// every shared screen for everyone. Toggleable via server settings.
	public boolean screenSharingEnabled = true;

	
	@Override
	public void onInitialize() {
		instance = this;
		WindowItem.register();
		WaylandCraftNetworking.register();
		
		ServerTickEvents.START_LEVEL_TICK.register(serverItemManager);
	}
	
}
