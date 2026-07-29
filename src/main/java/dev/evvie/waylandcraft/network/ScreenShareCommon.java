package dev.evvie.waylandcraft.network;

/* Shared constants for the screen sharing feature.
 *
 * Screen frames are captured on the sharing client, JPEG-compressed and then
 * split into chunks because Minecraft's custom payloads have a practical size
 * limit (the vanilla protocol rejects packets above ~2 MiB and large packets
 * cause noticeable lag). Chunking keeps individual packets small and lets the
 * server relay them without buffering whole frames in one packet.
 */
public final class ScreenShareCommon {

	private ScreenShareCommon() {}

	// Maximum payload bytes per frame chunk packet. Kept well below the vanilla
	// packet size limit so a single chunk never gets rejected.
	public static final int MAX_CHUNK_BYTES = 24 * 1024;

	// Hard cap on a single (compressed) frame. Frames larger than this are
	// dropped rather than flooding the network. At JPEG quality this comfortably
	// fits a downscaled desktop window.
	public static final int MAX_FRAME_BYTES = 512 * 1024;

	// Maximum dimension (width or height) a shared frame is downscaled to before
	// encoding. Limits bandwidth regardless of the source window resolution.
	public static final int MAX_FRAME_DIMENSION = 640;

	// A shared stream is considered stale (sender stopped without notifying) if
	// no frame arrives within this many milliseconds.
	public static final long STREAM_TIMEOUT_MS = 5000;
}
