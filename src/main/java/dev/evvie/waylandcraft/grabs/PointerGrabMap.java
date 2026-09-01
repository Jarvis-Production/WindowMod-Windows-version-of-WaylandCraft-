package dev.evvie.waylandcraft.grabs;

import java.util.ArrayList;

import org.jetbrains.annotations.Nullable;

import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.WindowDisplay;
import dev.evvie.waylandcraft.WindowDisplay.DisplayHitResult;
import dev.evvie.waylandcraft.bridge.WLCAbstractWindow;
import dev.evvie.waylandcraft.bridge.WLCSurface;
import net.minecraft.world.phys.Vec3;

public class PointerGrabMap {
	
	private WaylandCraft wlc;
	private PointerGrab exclusiveGrab = null;
	private ImplicitGrabs implicitGrabs = null;
	
	public PointerGrabMap(WaylandCraft wlc) {
		this.wlc = wlc;
	}
	
	public boolean isGrabActive() {
		return implicitGrabs != null || exclusiveGrab != null;
	}
	
	public boolean isExclusiveGrabActive() {
		return exclusiveGrab != null;
	}
	
	public boolean isGrabActive(int button) {
		return (exclusiveGrab != null && exclusiveGrab.button == button) || (implicitGrabs != null && implicitGrabs.contains(button));
	}
	
	// Start implicit pointer grab on a surface. Surface MUST have active pointer focus!
	public void startImplicit(DisplayHitResult hitResult, int button) {
		if(isExclusiveGrabActive()) return;
		
		if(implicitGrabs == null) implicitGrabs = new ImplicitGrabs(hitResult);
		if(implicitGrabs.contains(button)) return;
		
		int serial = wlc.bridge.sendButton(0x110 + button, 1);
		implicitGrabs.add(button, serial);
	}
	
	public void startExclusive(PointerGrab grab) {
		if(isExclusiveGrabActive()) return;
		
		this.releaseImplicit();
		
		try {
			grab.init();
		} catch (GrabDroppedException e) {
			return;
		}
		
		exclusiveGrab = grab;
	}
	
	public void moveWorld(Vec3 pos, Vec3 view, Vec3 up) {
		if(exclusiveGrab != null) {
			try {
				exclusiveGrab.moveWorld(pos, view, up);
			} catch(GrabDroppedException e) {
				exclusiveGrab = null;
			}
			
			return;
		}
		
		if(implicitGrabs == null) return;
		
		DisplayHitResult hitResult = implicitGrabs.window.intersect(pos, view);
		if(hitResult == null) return;
		
		Vec3 relativeCoords = hitResult.surfaceLocalOrigin.subtract(implicitGrabs.surface.xSubpos, implicitGrabs.surface.ySubpos, 0);
		wlc.bridge.sendMotion(relativeCoords.x, relativeCoords.y);
	}
	
	public void hover(WLCAbstractWindow window, WLCSurface surface, double x, double y) {
		if(exclusiveGrab != null) {
			try {
				exclusiveGrab.hover(window, surface, x, y);
			} catch(GrabDroppedException e) {
				exclusiveGrab = null;
			}
		}
	}
	
	public void hoverNone() {
		if(exclusiveGrab != null) {
			try {
				exclusiveGrab.hoverNone();
			} catch(GrabDroppedException e) {
				exclusiveGrab = null;
			}
		}
	}
	
	public void release(int button) {
		if(exclusiveGrab != null && exclusiveGrab.button == button) {
			try {
				exclusiveGrab.release();
			} catch (GrabDroppedException e) {
				// No handling necessary, grab always removed
			}
			exclusiveGrab = null;
			return;
		}
		
		if(implicitGrabs == null) return;
		
		if(implicitGrabs.contains(button)) {
			wlc.bridge.sendButton(0x110 + button, 0);
			implicitGrabs.remove(button);
		}
		
		if(implicitGrabs.isEmpty()) implicitGrabs = null;
	}
	
	private void releaseImplicit() {
		if(implicitGrabs == null) return;
		
		for(ImplicitGrab press : implicitGrabs.entries) {
			wlc.bridge.sendButton(0x110 + press.button, 0);
		}
		implicitGrabs = null;
	}
	
	public void releaseAll() {
		this.releaseImplicit();
		
		if(exclusiveGrab == null) return;
		
		try {
			exclusiveGrab.release();
		} catch (GrabDroppedException e) {
			// No handling necessary, grab always removed
		}
		exclusiveGrab = null;
	}
	
	/* Drop an active implicit pointer grab that matches the given serial.
	 * This method is supposed to be used to upgrade implicit grabs to exclusive ones.
	 * A button release event is not forwarded to the client.
	 */
	public @Nullable ImplicitGrab dropImplicitMatching(int serial) {
		if(isExclusiveGrabActive()) return null;
		if(implicitGrabs == null) return null;
		
		for(ImplicitGrab implicitGrab : implicitGrabs.entries) {
			if(implicitGrab.serial == serial) {
				implicitGrabs.remove(implicitGrab.button); // Warning: Could set implicitGrabs to null
				return implicitGrab;
			}
		}
		return null;
	}
	
	private static class ImplicitGrabs {
		
		public final WindowDisplay window;
		public final WLCSurface surface;
		public final Vec3 startWorldPos;
		public final Vec3 startSurfaceLocal;
		public ArrayList<ImplicitGrab> entries = new ArrayList<ImplicitGrab>();
		
		public ImplicitGrabs(DisplayHitResult hitResult) {
			this.window = hitResult.target;
			this.surface = hitResult.surface;
			this.startWorldPos = hitResult.position;
			this.startSurfaceLocal = hitResult.surfaceLocalOrigin;
		}
		
		public boolean contains(int button) {
			return entries.stream().anyMatch((press) -> press.button == button);
		}
		
		public boolean isEmpty() {
			return entries.isEmpty();
		}
		
		public void add(int button, int serial) {
			assert !contains(button);
			entries.add(new ImplicitGrab(window, surface, button, serial, startWorldPos, startSurfaceLocal));
		}
		
		public void remove(int button) {
			assert contains(button);
			entries.removeIf((press) -> press.button == button);
		}
		
	}
	
	// Not a real pointer grab, just a way to represent active button presses on a WindowDisplay
	public static record ImplicitGrab(WindowDisplay window, WLCSurface surface, int button, int serial, Vec3 startWorldPos, Vec3 startSurfaceLocal) {}
	
}
