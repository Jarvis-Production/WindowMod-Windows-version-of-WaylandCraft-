package dev.evvie.waylandcraft.network;

import dev.evvie.waylandcraft.WaylandCraftCommon;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

/* Sent by a sharing client to the server for every frame chunk.
 *
 * A logical frame is identified by (streamId, frameSeq). Because a compressed
 * frame may exceed the per-packet limit it is split into `chunkCount` chunks,
 * each carrying its `chunkIndex` and raw bytes. The receiver reassembles them
 * once all chunks for a frame arrive.
 *
 * `handle` is the native window handle being shared, so receivers can label the
 * shared display and the server can enforce ownership.
 */
public record ServerboundScreenFramePayload(
		long handle,
		int streamId,
		int frameSeq,
		int width,
		int height,
		int chunkIndex,
		int chunkCount,
		byte[] data) implements CustomPacketPayload {

	public static final Identifier ID = Identifier.fromNamespaceAndPath(WaylandCraftCommon.MOD_ID, "screen_frame_c2s");

	public static final CustomPacketPayload.Type<ServerboundScreenFramePayload> TYPE = new CustomPacketPayload.Type<>(ID);

	public static final StreamCodec<RegistryFriendlyByteBuf, ServerboundScreenFramePayload> CODEC = StreamCodec.composite(
			ByteBufCodecs.VAR_LONG, ServerboundScreenFramePayload::handle,
			ByteBufCodecs.VAR_INT, ServerboundScreenFramePayload::streamId,
			ByteBufCodecs.VAR_INT, ServerboundScreenFramePayload::frameSeq,
			ByteBufCodecs.VAR_INT, ServerboundScreenFramePayload::width,
			ByteBufCodecs.VAR_INT, ServerboundScreenFramePayload::height,
			ByteBufCodecs.VAR_INT, ServerboundScreenFramePayload::chunkIndex,
			ByteBufCodecs.VAR_INT, ServerboundScreenFramePayload::chunkCount,
			ByteBufCodecs.BYTE_ARRAY, ServerboundScreenFramePayload::data,
			ServerboundScreenFramePayload::new);

	@Override
	public Type<? extends CustomPacketPayload> type() {
		return TYPE;
	}
}
