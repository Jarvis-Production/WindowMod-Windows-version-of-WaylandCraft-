package dev.evvie.waylandcraft.grabs;

import dev.evvie.waylandcraft.displays.WindowDisplay;
import net.minecraft.world.phys.Vec3;

public class WindowGrab extends PointerGrab {
	
	private final WindowDisplay window;
	
	public WindowGrab(WindowDisplay window, int button) {
		super(button);
		this.window = window;
		window.anchorDistance = 2.0;
	}
	
	private void checkValid() throws GrabDroppedException {
		if(!window.isValid()) {
			this.drop();
		}
	}
	
	@Override
	public void init() throws GrabDroppedException {
		this.checkValid();
	}
	
	@Override
	public void release(boolean force) throws GrabDroppedException {
		this.checkValid();
	}
	
	@Override
	public void moveWorld(Vec3 pos, Vec3 view, Vec3 up, float yRot, float xRot) throws GrabDroppedException {
		this.checkValid();
		
		window.doGrabMove(pos, view, up, yRot);
	}

	@Override
	public void onScroll(double scrollX, double scrollY) throws GrabDroppedException {
		this.checkValid();

		window.adjustAnchorDistance(scrollY);
	}

}
