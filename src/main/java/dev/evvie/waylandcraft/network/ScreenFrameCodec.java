package dev.evvie.waylandcraft.network;

import java.awt.image.BufferedImage;
import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.util.Iterator;

import javax.imageio.IIOImage;
import javax.imageio.ImageIO;
import javax.imageio.ImageWriteParam;
import javax.imageio.ImageWriter;
import javax.imageio.stream.ImageOutputStream;
import javax.imageio.stream.MemoryCacheImageOutputStream;

import org.jetbrains.annotations.Nullable;
import org.lwjgl.opengl.GL30;
import org.lwjgl.system.MemoryUtil;

import com.mojang.blaze3d.opengl.GlStateManager;

import dev.evvie.waylandcraft.WaylandCraftCommon;

/* Helpers to turn a rendered window texture into a compressed JPEG frame and
 * back into ARGB pixels.
 *
 * Capture reads the OpenGL color texture into a CPU buffer, box-downscales it
 * to at most {@link ScreenShareCommon#MAX_FRAME_DIMENSION} on its longest side
 * to bound bandwidth, and JPEG-encodes it. Decoding reverses the JPEG step and
 * returns straight ARGB int pixels ready to upload into a texture.
 *
 * All GL calls must run on the render thread; encoding/decoding are pure CPU
 * and safe to run off-thread.
 */
public final class ScreenFrameCodec {

	private ScreenFrameCodec() {}

	public record CapturedFrame(int width, int height, byte[] jpeg) {}

	public record DecodedFrame(int width, int height, int[] argb) {}

	/* Read `glTextureId` (a width x height RGBA8 texture), downscale and encode
	 * to JPEG. Must be called on the render thread. Returns null on failure.
	 */
	public static @Nullable CapturedFrame capture(int glTextureId, int srcWidth, int srcHeight, float quality) {
		if(glTextureId <= 0 || srcWidth <= 0 || srcHeight <= 0) return null;

		// Read the full texture into a native buffer via glGetTexImage. This is
		// simple and version-stable; the texture is RGBA8 (see BufferTexture /
		// TextureTarget which both allocate RGBA8 color attachments).
		int pixelCount = srcWidth * srcHeight;
		ByteBuffer buf = MemoryUtil.memAlloc(pixelCount * 4);
		try {
			GlStateManager._bindTexture(glTextureId);
			GlStateManager._pixelStore(GL30.GL_PACK_ALIGNMENT, 1);
			GL30.glGetTexImage(GL30.GL_TEXTURE_2D, 0, GL30.GL_RGBA, GL30.GL_UNSIGNED_BYTE, buf);

			// Compute downscaled dimensions preserving aspect ratio.
			int maxDim = ScreenShareCommon.MAX_FRAME_DIMENSION;
			int dstWidth = srcWidth;
			int dstHeight = srcHeight;
			if(srcWidth > maxDim || srcHeight > maxDim) {
				double scale = Math.min((double) maxDim / srcWidth, (double) maxDim / srcHeight);
				dstWidth = Math.max(1, (int) Math.round(srcWidth * scale));
				dstHeight = Math.max(1, (int) Math.round(srcHeight * scale));
			}

			// JPEG has no alpha, so build an RGB BufferedImage while downscaling
			// with a simple nearest-neighbour sample (cheap and good enough for
			// a downscaled desktop stream).
			BufferedImage image = new BufferedImage(dstWidth, dstHeight, BufferedImage.TYPE_INT_RGB);
			for(int y = 0; y < dstHeight; y++) {
				int srcY = (int) ((long) y * srcHeight / dstHeight);
				for(int x = 0; x < dstWidth; x++) {
					int srcX = (int) ((long) x * srcWidth / dstWidth);
					int idx = (srcY * srcWidth + srcX) * 4;
					int r = buf.get(idx) & 0xFF;
					int g = buf.get(idx + 1) & 0xFF;
					int b = buf.get(idx + 2) & 0xFF;
					image.setRGB(x, y, (r << 16) | (g << 8) | b);
				}
			}

			byte[] jpeg = encodeJpeg(image, quality);
			if(jpeg == null || jpeg.length > ScreenShareCommon.MAX_FRAME_BYTES) {
				return null;
			}
			return new CapturedFrame(dstWidth, dstHeight, jpeg);
		} catch(Exception e) {
			WaylandCraftCommon.LOGGER.error("Screen share capture failed", e);
			return null;
		} finally {
			MemoryUtil.memFree(buf);
		}
	}

	private static @Nullable byte[] encodeJpeg(BufferedImage image, float quality) throws IOException {
		Iterator<ImageWriter> writers = ImageIO.getImageWritersByFormatName("jpg");
		if(!writers.hasNext()) return null;
		ImageWriter writer = writers.next();
		try {
			ByteArrayOutputStream out = new ByteArrayOutputStream();
			try(ImageOutputStream ios = new MemoryCacheImageOutputStream(out)) {
				writer.setOutput(ios);
				ImageWriteParam param = writer.getDefaultWriteParam();
				if(param.canWriteCompressed()) {
					param.setCompressionMode(ImageWriteParam.MODE_EXPLICIT);
					param.setCompressionQuality(Math.clamp(quality, 0.1f, 1.0f));
				}
				writer.write(null, new IIOImage(image, null, null), param);
			}
			return out.toByteArray();
		} finally {
			writer.dispose();
		}
	}

	/* Decode a JPEG frame into opaque ARGB pixels. Safe to run off-thread. */
	public static @Nullable DecodedFrame decode(byte[] jpeg) {
		try {
			BufferedImage image = ImageIO.read(new ByteArrayInputStream(jpeg));
			if(image == null) return null;
			int w = image.getWidth();
			int h = image.getHeight();
			int[] rgb = image.getRGB(0, 0, w, h, null, 0, w);
			for(int i = 0; i < rgb.length; i++) {
				rgb[i] = 0xFF000000 | (rgb[i] & 0x00FFFFFF);
			}
			return new DecodedFrame(w, h, rgb);
		} catch(Exception e) {
			WaylandCraftCommon.LOGGER.error("Screen share decode failed", e);
			return null;
		}
	}
}
