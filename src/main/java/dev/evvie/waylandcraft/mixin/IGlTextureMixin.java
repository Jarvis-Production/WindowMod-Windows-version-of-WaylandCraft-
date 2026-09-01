package dev.evvie.waylandcraft.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Invoker;

import com.mojang.blaze3d.opengl.GlTexture;
import com.mojang.blaze3d.textures.TextureFormat;

@Mixin(GlTexture.class)
public interface IGlTextureMixin {
	
	@Invoker("<init>")
	static GlTexture createTexture(int usage, String string, TextureFormat textureFormat, int width, int height, int depthOrLayers, int mipLevels, int id) {
		throw new AssertionError();
	}
	
}
