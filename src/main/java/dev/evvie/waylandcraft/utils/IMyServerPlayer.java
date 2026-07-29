package dev.evvie.waylandcraft.utils;

import java.util.ArrayList;

public interface IMyServerPlayer {
	
	void setItemGiveCooldown(int cooldown);
	int getItemGiveCooldown();
	
	ArrayList<Long> getAliveWindows();
	
	// Whether other players are permitted to grab (take into their own world as
	// an item) the windows this player is sharing. Controlled by the sharer via
	// their client settings and synced to the server.
	boolean isScreenShareGrabAllowed();
	void setScreenShareGrabAllowed(boolean allowed);
	
}

