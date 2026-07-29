package dev.evvie.waylandcraft.network;

import java.util.UUID;

import dev.evvie.waylandcraft.WaylandCraftCommon;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

/* Relayed by the server to tell every player that a shared stream has ended
 * (sender stopped sharing, disconnected, or the server disabled sharing).
 */
public record ClientboundStopSharePayload(UUID sharer, int streamId) implements CustomPacketPayload {

	public static final Identifier ID = Identifier.fromNamespaceAndPath(WaylandCraftCommon.MOD_ID, "screen_stop_s2c");

	public static final CustomPacketPayload.Type<ClientboundStopSharePayload> TYPE = new CustomPacketPayload.Type<>(ID);

	public static final StreamCodec<RegistryFriendlyByteBuf, ClientboundStopSharePayload> CODEC = StreamCodec.composite(
			UUIDUtil.STREAM_CODEC, ClientboundStopSharePayload::sharer,
			ByteBufCodecs.VAR_INT, ClientboundStopSharePayload::streamId,
			ClientboundStopSharePayload::new);

	@Override
	public Type<? extends CustomPacketPayload> type() {
		return TYPE;
	}
}
