package dev.evvie.waylandcraft.bridge;

import org.jetbrains.annotations.Nullable;

public class WLCPopup extends WLCAbstractWindow {
	
	@Nullable
	protected WLCAbstractWindow parent = null;
	
	protected long parentHandle = 0;
	
	public int offsetX = 0;
	public int offsetY = 0;
	
	public WLCPopup(long handle) {
		super(handle);
	}
	
	public WLCAbstractWindow getParent() {
		return parent;
	}
	
}
