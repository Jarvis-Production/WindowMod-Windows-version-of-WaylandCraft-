package dev.evvie.waylandcraft.grabs;

import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.bridge.WLCAbstractWindow;
import dev.evvie.waylandcraft.bridge.WLCSurface;
import net.minecraft.world.phys.Vec3;

public abstract class PointerGrab {
	
	public final WaylandCraft wlc;
	public final int button;
	
	public PointerGrab(int button) {
		this.button = button;
		this.wlc = WaylandCraft.instance;
	}
	
	public void drop() throws GrabDroppedException {
		throw new GrabDroppedException();
	}
	
	// Called when grab is first started
	public abstract void init() throws GrabDroppedException;
	
	// Called when button is released
	public abstract void release() throws GrabDroppedException;
	
	// Called every time the pointer is moved in the world. Arguments are world position, view vector and view up vector
	public abstract void moveWorld(Vec3 pos, Vec3 view, Vec3 up) throws GrabDroppedException;
	
	// Called every time the pointer is moved over a window, coordinates relative to given (sub-)surface
	public void hover(WLCAbstractWindow window, WLCSurface surface, double x, double y) throws GrabDroppedException {}
	
	// Called every time the pointer is moved in the world but does not hover any surfaces
	public void hoverNone() throws GrabDroppedException {}
	
}
