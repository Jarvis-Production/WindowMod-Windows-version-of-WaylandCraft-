package dev.evvie.waylandcraft.network;

import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;

import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.WaylandCraftCommon;
import dev.evvie.waylandcraft.bridge.WLCToplevel;
import dev.evvie.waylandcraft.render.WindowFramebuffer;
import net.fabricmc.fabric.api.client.networking.v1.ClientPlayNetworking;
import net.minecraft.client.Minecraft;

/* Client-side sender for screen sharing.
 *
 * The player can toggle sharing of a specific window. While a window is shared
 * this manager periodically (every FRAME_INTERVAL_TICKS) grabs its rendered
 * framebuffer on the render thread, hands the pixels to a background thread for
 * JPEG encoding, then splits the result into chunk packets and sends them to
 * the server which relays them to everyone.
 *
 * We use one worker thread and skip capturing a new frame while the previous
 * one is still encoding, so a slow encode can never build an unbounded backlog.
 */
public class ScreenShareManager {

	private static final int FRAME_INTERVAL_TICKS = 4; // ~5 fps at 20 tps
	private static final float JPEG_QUALITY = 0.5f;

	private final ExecutorService encoder = Executors.newSingleThreadExecutor(r -> {
		Thread t = new Thread(r, "waylandcraft-screenshare-encoder");
		t.setDaemon(true);
		return t;
	});

	// Active outgoing streams keyed by native window handle.
	private final Map<Long, Stream> streams = new HashMap<>();
	private final AtomicInteger nextStreamId = new AtomicInteger(1);

	private int tickCounter = 0;

	private static class Stream {
		final int streamId;
		final long handle;
		int frameSeq = 0;
		boolean allowGrab;
		final AtomicBoolean encoding = new AtomicBoolean(false);

		Stream(int streamId, long handle, boolean allowGrab) {
			this.streamId = streamId;
			this.handle = handle;
			this.allowGrab = allowGrab;
		}
	}

	public boolean isSharing(long handle) {
		return streams.containsKey(handle);
	}

	public boolean isSharingAny() {
		return !streams.isEmpty();
	}

	/* Toggle sharing of the given window. Returns true if now sharing. */
	public boolean toggleShare(WLCToplevel toplevel, boolean allowGrab) {
		long handle = toplevel.getHandle();
		if(streams.containsKey(handle)) {
			stopShare(handle);
			return false;
		}
		int id = nextStreamId.getAndIncrement();
		streams.put(handle, new Stream(id, handle, allowGrab));
		WaylandCraftCommon.LOGGER.info("Started sharing window 0x" + Long.toHexString(handle) + " as stream " + id);
		return true;
	}

	public void stopShare(long handle) {
		Stream stream = streams.remove(handle);
		if(stream == null) return;
		ClientPlayNetworking.send(new ServerboundStopSharePayload(stream.streamId));
		WaylandCraftCommon.LOGGER.info("Stopped sharing stream " + stream.streamId);
	}

	public void stopAll() {
		for(long handle : new java.util.ArrayList<>(streams.keySet())) {
			stopShare(handle);
		}
	}

	/* Called every client tick. Captures and sends frames for active streams. */
	public void tick() {
		if(streams.isEmpty()) return;
		if(WaylandCraft.instance == null || WaylandCraft.instance.bridge == null) return;
		if(Minecraft.getInstance().getConnection() == null) return;

		tickCounter++;
		if(tickCounter < FRAME_INTERVAL_TICKS) return;
		tickCounter = 0;

		for(Stream stream : new java.util.ArrayList<>(streams.values())) {
			WLCToplevel toplevel = WaylandCraft.instance.bridge.getToplevel(stream.handle);
			if(toplevel == null || !toplevel.isAlive()) {
				// The shared window closed; tell the server and drop the stream.
				stopShare(stream.handle);
				continue;
			}
			captureAndSend(stream, toplevel);
		}
	}

	private void captureAndSend(Stream stream, WLCToplevel toplevel) {
		// Skip if the previous frame for this stream is still encoding.
		if(stream.encoding.get()) return;

		WindowFramebuffer fb = toplevel.framebuffer;
		if(fb == null || !fb.isValid()) return;

		int glId = fb.getColorTextureGlId();
		int width = fb.getWidth();
		int height = fb.getHeight();
		if(glId <= 0 || width <= 0 || height <= 0) return;

		// Capture pixels on the render thread (current thread is the client
		// thread which owns the GL context during tick on this integrated path;
		// capture() only issues GL reads which are valid here).
		ScreenFrameCodec.CapturedFrame frame = ScreenFrameCodec.capture(glId, width, height, JPEG_QUALITY);
		if(frame == null) return;

		final int seq = stream.frameSeq++;
		stream.encoding.set(true);
		// Chunking + sending is cheap; do it on the worker to keep the client
		// thread free, then release the encoding gate.
		encoder.submit(() -> {
			try {
				sendChunks(stream, seq, frame);
			} finally {
				stream.encoding.set(false);
			}
		});
	}

	private void sendChunks(Stream stream, int seq, ScreenFrameCodec.CapturedFrame frame) {
		byte[] data = frame.jpeg();
		int chunkSize = ScreenShareCommon.MAX_CHUNK_BYTES;
		int chunkCount = Math.max(1, (data.length + chunkSize - 1) / chunkSize);

		for(int i = 0; i < chunkCount; i++) {
			int start = i * chunkSize;
			int end = Math.min(start + chunkSize, data.length);
			byte[] chunk = new byte[end - start];
			System.arraycopy(data, start, chunk, 0, chunk.length);

			ServerboundScreenFramePayload payload = new ServerboundScreenFramePayload(
					stream.handle, stream.streamId, seq,
					frame.width(), frame.height(), i, chunkCount, chunk);

			// Networking must be invoked on the client thread.
			Minecraft.getInstance().execute(() -> {
				if(Minecraft.getInstance().getConnection() != null) {
					ClientPlayNetworking.send(payload);
				}
			});
		}
	}
}
