package dev.evvie.waylandcraft.network;

import org.jetbrains.annotations.Nullable;

/* Reassembles a single logical frame from its chunks.
 *
 * Chunks of a frame all share the same frameSeq. As soon as a chunk with a
 * newer frameSeq arrives we discard any partial older frame (frames are only
 * useful whole and the newest one supersedes older ones). Returns the complete
 * JPEG byte array once every chunk of a frame has been collected.
 */
public class FrameReassembler {

	private int currentSeq = -1;
	private int chunkCount = 0;
	private int received = 0;
	private byte[][] chunks = null;

	public @Nullable byte[] accept(int frameSeq, int chunkIndex, int totalChunks, byte[] data) {
		if(totalChunks <= 0 || chunkIndex < 0 || chunkIndex >= totalChunks) return null;

		// Ignore chunks belonging to a frame older than the one we're building.
		if(frameSeq < currentSeq) return null;

		if(frameSeq != currentSeq) {
			// Start collecting a new frame; drop any incomplete previous one.
			currentSeq = frameSeq;
			chunkCount = totalChunks;
			chunks = new byte[totalChunks][];
			received = 0;
		}

		if(chunks == null || chunkIndex >= chunks.length) return null;
		if(chunks[chunkIndex] == null) {
			chunks[chunkIndex] = data;
			received++;
		}

		if(received < chunkCount) return null;

		// Concatenate all chunks in order into the full frame.
		int total = 0;
		for(byte[] c : chunks) total += c.length;
		byte[] full = new byte[total];
		int pos = 0;
		for(byte[] c : chunks) {
			System.arraycopy(c, 0, full, pos, c.length);
			pos += c.length;
		}

		// Reset so the same seq isn't emitted twice.
		chunks = null;
		received = 0;
		return full;
	}
}
