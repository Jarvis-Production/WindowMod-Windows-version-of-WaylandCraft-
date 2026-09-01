package dev.evvie.waylandcraft.render;

import java.util.ArrayList;
import java.util.OptionalInt;

import com.mojang.blaze3d.buffers.GpuBuffer;
import com.mojang.blaze3d.buffers.Std140Builder;
import com.mojang.blaze3d.buffers.Std140SizeCalculator;
import com.mojang.blaze3d.pipeline.BlendFunction;
import com.mojang.blaze3d.pipeline.RenderPipeline;
import com.mojang.blaze3d.pipeline.RenderTarget;
import com.mojang.blaze3d.pipeline.TextureTarget;
import com.mojang.blaze3d.platform.DestFactor;
import com.mojang.blaze3d.platform.SourceFactor;
import com.mojang.blaze3d.shaders.UniformType;
import com.mojang.blaze3d.systems.RenderPass;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.textures.GpuTextureView;
import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexFormat;

import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.bridge.WLCSurface;
import dev.evvie.waylandcraft.bridge.WLCSurface.ViewportSource;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.MappableRingBuffer;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.texture.AbstractTexture;
import net.minecraft.client.renderer.texture.MissingTextureAtlasSprite;
import net.minecraft.client.renderer.texture.TextureManager;
import net.minecraft.resources.Identifier;

public class WindowFramebuffer {
	
	public static final RenderPipeline WINDOW_PIPELINE = RenderPipelines.register(
		RenderPipeline.builder()
		.withLocation(Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "pipeline/window"))
		.withVertexShader(Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "window"))
		.withFragmentShader(Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "window"))
		.withVertexFormat(DefaultVertexFormat.POSITION_TEX, VertexFormat.Mode.QUADS)
		.withSampler("sampler")
		.withUniform("window_info", UniformType.UNIFORM_BUFFER)
		.withBlend(new BlendFunction(SourceFactor.ONE, DestFactor.ONE_MINUS_SRC_ALPHA))
		.withCull(false)
		.build()
	);
	
	public final WLCSurface surfaceTree;
	private RenderTarget target = null;
	private FramebufferTexture texture = null;
	private Identifier location = null;
	
	private int width = 0;
	private int height = 0;
	private int xoff;
	private int yoff;
	
	private WindowFramebuffer(WLCSurface surfaceTree) {
		this.surfaceTree = surfaceTree;
	}
	
	public static WindowFramebuffer renderSurfaceTree(WLCSurface surfaceTree) {
		WindowFramebuffer buf = new WindowFramebuffer(surfaceTree);
		buf.init();
		return buf;
	}
	
	private void init() {
		updateDimensions();
		render();
		registerTexture();
	}
	
	private void updateDimensions() {
		int minX = 0;
		int minY = 0;
		int maxX = 0;
		int maxY = 0;
		
		for(WLCSurface surface = surfaceTree; surface != null; surface = surface.getNextChild()) {
			int sMinX = surface.xSubpos;
			int sMinY = surface.ySubpos;
			int sMaxX = sMinX + surface.width();
			int sMaxY = sMinY + surface.height();
			
			if(sMinX < minX) minX = sMinX;
			if(sMinY < minY) minY = sMinY;
			if(sMaxX > maxX) maxX = sMaxX;
			if(sMaxY > maxY) maxY = sMaxY;
		}
		
		this.xoff = -minX;
		this.yoff = -minY;
		this.width = maxX - minX;
		this.height = maxY - minY;
	}
	
	private String name() {
		return "wayland-framebuffer-" + this.hashCode() + "-" + surfaceTree.hashCode();
	}
	
	private void render() {
		if(width == 0 || height == 0) return;
		
		target = new TextureTarget(name(), width, height, false);
		
		PoseStack poseStack = new PoseStack();
		poseStack.translate(-1.0, -1.0, 0.0);
		poseStack.scale(2.0f / width, 2.0f / height, 1.0f);
		
		ArrayList<CompiledBufferDraw> elements = new ArrayList<>();
		for(WLCSurface surface = surfaceTree; surface != null; surface = surface.getNextChild()) {
			BufferDraw draw = bakeSurface(surface, xoff + surface.xSubpos, yoff + surface.ySubpos);
			if(draw != null) elements.add(draw.compile());
		}
		
		MappableRingBuffer alphaUniforms = new MappableRingBuffer(() -> "framebuffer uniforms", GpuBuffer.USAGE_UNIFORM | GpuBuffer.USAGE_MAP_WRITE, new Std140SizeCalculator().putMat4f().putFloat().get());
		alphaUniforms.rotate();
		try(GpuBuffer.MappedView view = RenderSystem.getDevice().createCommandEncoder().mapBuffer(alphaUniforms.currentBuffer(), false, true)) {
			Std140Builder.intoBuffer(view.data()).putMat4f(poseStack.last().pose()).putFloat(0.0f);
		}
		
		MappableRingBuffer nonAlphaUniforms = new MappableRingBuffer(() -> "framebuffer uniforms", GpuBuffer.USAGE_UNIFORM | GpuBuffer.USAGE_MAP_WRITE, new Std140SizeCalculator().putMat4f().putFloat().get());
		nonAlphaUniforms.rotate();
		try(GpuBuffer.MappedView view = RenderSystem.getDevice().createCommandEncoder().mapBuffer(nonAlphaUniforms.currentBuffer(), false, true)) {
			Std140Builder.intoBuffer(view.data()).putMat4f(poseStack.last().pose()).putFloat(1.0f);
		}
		
		try(RenderPass pass = RenderSystem.getDevice().createCommandEncoder().createRenderPass(() -> "window framebuffer", target.getColorTextureView(), OptionalInt.of(0x00000000))) {
			pass.setPipeline(WINDOW_PIPELINE);
			for(CompiledBufferDraw element : elements) {
				pass.setUniform("window_info", element.alpha ? alphaUniforms.currentBuffer() : nonAlphaUniforms.currentBuffer());
				pass.bindTexture("sampler", element.textureView, RenderUtils.WINDOW_SAMPLER.get());
				pass.setVertexBuffer(0, element.vertexBuffer);
				pass.setIndexBuffer(element.indexBuffer, element.indexType);
				pass.drawIndexed(0, 0, element.indexCount, 1);
			}
		}
		
		alphaUniforms.close();
		nonAlphaUniforms.close();
	}
	
	private BufferDraw bakeSurface(WLCSurface surface, float x, float y) {
		BufferTexture buf = surface.getBuffer();
		if(buf == null) return null;
		
		float w = surface.width();
		float h = surface.height();
		
		float crop_x1 = 0.0f;
		float crop_y1 = 0.0f;
		float crop_x2 = 1.0f;
		float crop_y2 = 1.0f;
		
		ViewportSource src = surface.getViewportSource();
		if(src != null) {
			crop_x1 = (float) (src.x / buf.width);
			crop_y1 = (float) (src.y / buf.height);
			crop_x2 = (float) ((src.x + src.width) / buf.width);
			crop_y2 = (float) ((src.y + src.height) / buf.height);
		}
		
		return new BufferDraw(buf.textureView, x, y, w, h, crop_x1, crop_y1, crop_x2, crop_y2, buf.format != BufferTexture.FORMAT_XRGB8888);
	}
	
	private static record CompiledBufferDraw(GpuTextureView textureView, GpuBuffer vertexBuffer, GpuBuffer indexBuffer, int indexCount, VertexFormat.IndexType indexType, boolean alpha) {
	}
	
	private static record BufferDraw(GpuTextureView textureView, float x, float y, float w, float h, float u1, float v1, float u2, float v2, boolean alpha) {
		
		public CompiledBufferDraw compile() {
			try(ByteBufferBuilder byteBuilder = new ByteBufferBuilder(DefaultVertexFormat.POSITION_TEX.getVertexSize() * 4)) {
				BufferBuilder builder = new BufferBuilder(byteBuilder, VertexFormat.Mode.QUADS, DefaultVertexFormat.POSITION_TEX);
				builder.addVertex(x, y, 0).setUv(u1, v1);
				builder.addVertex(x + w, y, 0).setUv(u2, v1);
				builder.addVertex(x + w, y + h, 0).setUv(u2, v2);
				builder.addVertex(x, y + h, 0).setUv(u1, v2);
				
				try(MeshData mesh = builder.buildOrThrow()) {
					int indexCount = mesh.drawState().indexCount();
					RenderSystem.AutoStorageIndexBuffer indices = RenderSystem.getSequentialBuffer(VertexFormat.Mode.QUADS);
					GpuBuffer vertexBuffer = RenderSystem.getDevice().createBuffer(null, GpuBuffer.USAGE_VERTEX | GpuBuffer.USAGE_COPY_DST, mesh.vertexBuffer());
					GpuBuffer indexBuffer = indices.getBuffer(indexCount);
					return new CompiledBufferDraw(textureView, vertexBuffer, indexBuffer, indexCount, indices.type(), alpha);
				}
			}
		}
		
	}
	
	private void registerTexture() {
		if(target == null) return;
		
		texture = new FramebufferTexture(getTextureView());
		location = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, name());
		
		Minecraft.getInstance().getTextureManager().register(location, texture);
	}
	
	private void unregisterTexture() {
		TextureManager manager = Minecraft.getInstance().getTextureManager();
		manager.register(location, manager.getTexture(MissingTextureAtlasSprite.getLocation()));
	}
	
	public void free() {
		if(target != null) target.destroyBuffers();
		if(texture != null) unregisterTexture();
	}
	
	public int getWidth() {
		return width;
	}
	
	public int getHeight() {
		return height;
	}
	
	public int getXOff() {
		return xoff;
	}
	
	public int getYOff() {
		return yoff;
	}
	
	public GpuTextureView getTextureView() {
		if(target == null) return null;
		return target.getColorTextureView();
	}
	
	public Identifier getTextureLocation() {
		return location;
	}
	
	public boolean isValid() {
		return target != null;
	}
	
	private static class FramebufferTexture extends AbstractTexture {
		
		public FramebufferTexture(GpuTextureView textureView) {
			this.textureView = textureView;
			this.texture = textureView.texture();
		}
		
		@Override
		public void close() {
		}
		
	}
	
}
