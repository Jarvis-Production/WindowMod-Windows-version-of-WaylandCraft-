package dev.evvie.waylandcraft.network;

import dev.evvie.waylandcraft.WaylandCraftCommon;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

/* Sent by a client to inform the server of its screen-sharing preferences, so
 * the server can stamp relayed frames with the correct permissions. Currently
 * carries whether other players may grab the shared window as an item.
 */
public record ServerboundShareSettingsPayload(boolean allowGrab) implements CustomPacketPayload {

	public static final Identifier ID = Identifier.fromNamespaceAndPath(WaylandCraftCommon.MOD_ID, "share_settings_c2s");

	public static final CustomPacketPayload.Type<ServerboundShareSettingsPayload> TYPE = new CustomPacketPayload.Type<>(ID);

	public static final StreamCodec<RegistryFriendlyByteBuf, ServerboundShareSettingsPayload> CODEC = StreamCodec.composite(
			ByteBufCodecs.BOOL, ServerboundShareSettingsPayload::allowGrab,
			ServerboundShareSettingsPayload::new);

	@Override
	public Type<? extends CustomPacketPayload> type() {
		return TYPE;
	}
}
