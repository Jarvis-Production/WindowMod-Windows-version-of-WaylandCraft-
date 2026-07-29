package dev.evvie.waylandcraft.item;

import java.util.UUID;

import com.mojang.serialization.Codec;
import com.mojang.serialization.codecs.RecordCodecBuilder;

import io.netty.buffer.ByteBuf;

import net.minecraft.core.UUIDUtil;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.world.entity.player.Player;

public record WindowHandle(UUID player, long handle) {

	public static final Codec<WindowHandle> CODEC = RecordCodecBuilder.create(builder -> {
		return builder.group(
				UUIDUtil.CODEC.fieldOf("player").forGetter(WindowHandle::player),
				Codec.LONG.fieldOf("handle").forGetter(WindowHandle::handle)
		).apply(builder, WindowHandle::new);
	});

	public static final StreamCodec<ByteBuf, WindowHandle> STREAM_CODEC = StreamCodec.composite(
		UUIDUtil.STREAM_CODEC, WindowHandle::player,
		ByteBufCodecs.VAR_LONG, WindowHandle::handle,
		WindowHandle::new
	);
	
	public static WindowHandle forPlayer(Player player, long handle) {
		return new WindowHandle(getPlayerUUID(player), handle);
	}
	
	public static UUID getPlayerUUID(Player player) {
		return player.getGameProfile().id();
	}
	
	public boolean matchesPlayer(Player player) {
		return getPlayerUUID(player).equals(this.player());
	}
	
}
