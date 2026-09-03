package dev.evvie.waylandcraft.bridge;

import dev.evvie.waylandcraft.render.WindowFramebuffer;

public class ImageWindow extends WLCAbstractWindow {
	
	public final String imagePath;
	
	public ImageWindow(String imagePath, int width, int height, WindowFramebuffer framebuffer) {
		super(1);
		this.imagePath = imagePath;
		this.geometry = new SurfaceGeometry(0, 0, width, height);
		this.framebuffer = framebuffer;
	}
	
}
