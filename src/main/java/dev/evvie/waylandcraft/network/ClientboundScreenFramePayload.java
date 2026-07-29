package dev.evvie.waylandcraft.network;

import java.util.UUID;

import dev.evvie.waylandcraft.WaylandCraftCommon;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

/* Relayed by the server to every player for each frame chunk of a shared
 * stream. Mirrors the serverbound frame packet but adds the sharer's identity
 * and an `allowGrab` flag describing whether receivers may pull the window into
 * their own world as a takeable item.
 */
public record ClientboundScreenFramePayload(
		UUID sharer,
		long handle,
		int streamId,
		int frameSeq,
		int width,
		int height,
		int chunkIndex,
		int chunkCount,
		boolean allowGrab,
		byte[] data) implements CustomPacketPayload {

	public static final Identifier ID = Identifier.fromNamespaceAndPath(WaylandCraftCommon.MOD_ID, "screen_frame_s2c");

	public static final CustomPacketPayload.Type<ClientboundScreenFramePayload> TYPE = new CustomPacketPayload.Type<>(ID);

	public static final StreamCodec<RegistryFriendlyByteBuf, ClientboundScreenFramePayload> CODEC = StreamCodec.composite(
			UUIDUtil.STREAM_CODEC, ClientboundScreenFramePayload::sharer,
			ByteBufCodecs.VAR_LONG, ClientboundScreenFramePayload::handle,
			ByteBufCodecs.VAR_INT, ClientboundScreenFramePayload::streamId,
			ByteBufCodecs.VAR_INT, ClientboundScreenFramePayload::frameSeq,
			ByteBufCodecs.VAR_INT, ClientboundScreenFramePayload::width,
			ByteBufCodecs.VAR_INT, ClientboundScreenFramePayload::height,
			ByteBufCodecs.VAR_INT, ClientboundScreenFramePayload::chunkIndex,
			ByteBufCodecs.VAR_INT, ClientboundScreenFramePayload::chunkCount,
			ByteBufCodecs.BOOL, ClientboundScreenFramePayload::allowGrab,
			ByteBufCodecs.BYTE_ARRAY, ClientboundScreenFramePayload::data,
			ClientboundScreenFramePayload::new);

	@Override
	public Type<? extends CustomPacketPayload> type() {
		return TYPE;
	}
}
