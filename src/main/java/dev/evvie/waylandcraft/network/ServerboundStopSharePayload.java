package dev.evvie.waylandcraft.network;

import dev.evvie.waylandcraft.WaylandCraftCommon;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

/* Sent by a sharing client when it stops sharing a stream so the server can
 * promptly tell every receiver to remove the shared display.
 */
public record ServerboundStopSharePayload(int streamId) implements CustomPacketPayload {

	public static final Identifier ID = Identifier.fromNamespaceAndPath(WaylandCraftCommon.MOD_ID, "screen_stop_c2s");

	public static final CustomPacketPayload.Type<ServerboundStopSharePayload> TYPE = new CustomPacketPayload.Type<>(ID);

	public static final StreamCodec<RegistryFriendlyByteBuf, ServerboundStopSharePayload> CODEC = StreamCodec.composite(
			ByteBufCodecs.VAR_INT, ServerboundStopSharePayload::streamId,
			ServerboundStopSharePayload::new);

	@Override
	public Type<? extends CustomPacketPayload> type() {
		return TYPE;
	}
}
