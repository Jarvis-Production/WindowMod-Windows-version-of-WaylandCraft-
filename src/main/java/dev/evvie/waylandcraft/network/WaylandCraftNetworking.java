package dev.evvie.waylandcraft.network;

import java.util.ArrayList;

import dev.evvie.waylandcraft.WaylandCraftCommon;
import dev.evvie.waylandcraft.utils.IMyServerPlayer;
import net.fabricmc.fabric.api.networking.v1.PayloadTypeRegistry;
import net.fabricmc.fabric.api.networking.v1.ServerPlayNetworking;
import net.minecraft.server.level.ServerPlayer;

public class WaylandCraftNetworking {
	
	public static void register() {
		PayloadTypeRegistry.serverboundPlay().register(ServerboundGiveItemsPayload.TYPE, ServerboundGiveItemsPayload.CODEC);
		PayloadTypeRegistry.serverboundPlay().register(ServerboundAliveWindowsPayload.TYPE, ServerboundAliveWindowsPayload.CODEC);
		
		// Screen sharing packets. Serverbound frames/stops come from the sharing
		// client; clientbound frames/stops are relayed by the server to everyone.
		PayloadTypeRegistry.serverboundPlay().register(ServerboundScreenFramePayload.TYPE, ServerboundScreenFramePayload.CODEC);
		PayloadTypeRegistry.serverboundPlay().register(ServerboundStopSharePayload.TYPE, ServerboundStopSharePayload.CODEC);
		PayloadTypeRegistry.serverboundPlay().register(ServerboundShareSettingsPayload.TYPE, ServerboundShareSettingsPayload.CODEC);
		PayloadTypeRegistry.clientboundPlay().register(ClientboundScreenFramePayload.TYPE, ClientboundScreenFramePayload.CODEC);
		PayloadTypeRegistry.clientboundPlay().register(ClientboundStopSharePayload.TYPE, ClientboundStopSharePayload.CODEC);

		
		ServerPlayNetworking.registerGlobalReceiver(ServerboundGiveItemsPayload.TYPE, (payload, ctx) -> {
			IMyServerPlayer plr = (IMyServerPlayer) ctx.player();
			if(plr.getItemGiveCooldown() > 0) return;
			plr.setItemGiveCooldown(10);
			
			ArrayList<Long> handles = new ArrayList<Long>();
			for(long handle : payload.handles()) {
				if(handles.contains(handle)) continue;
				handles.add(handle);
			}
			
			if(payload.missingOnly()) WaylandCraftCommon.instance.serverItemManager.giveItemsIfMissing(ctx.player(), handles);
			else WaylandCraftCommon.instance.serverItemManager.giveItems(ctx.player(), handles);
		});
		
		ServerPlayNetworking.registerGlobalReceiver(ServerboundAliveWindowsPayload.TYPE, (payload, ctx) -> {
			IMyServerPlayer plr = (IMyServerPlayer) ctx.player();
			ArrayList<Long> handles = plr.getAliveWindows();
			handles.clear();
			
			for(long handle : payload.handles()) {
				handles.add(handle);
			}
		});
		
		// Relay a shared frame chunk to every player except the sharer itself
		// (the sharer already renders the window locally). Dropped entirely when
		// the server has screen sharing disabled.
		ServerPlayNetworking.registerGlobalReceiver(ServerboundScreenFramePayload.TYPE, (payload, ctx) -> {
			if(!WaylandCraftCommon.instance.screenSharingEnabled) return;
			
			// Basic sanity limits to avoid a malicious client flooding others.
			if(payload.chunkCount() < 1 || payload.chunkIndex() < 0 || payload.chunkIndex() >= payload.chunkCount()) return;
			if(payload.data().length > ScreenShareCommon.MAX_CHUNK_BYTES) return;
			
			ServerPlayer sharer = ctx.player();
			IMyServerPlayer plr = (IMyServerPlayer) sharer;
			// Ownership: only relay frames for a window the sharer actually owns
			// (its handle is in their alive-window set).
			if(!plr.getAliveWindows().contains(payload.handle())) return;
			
			ClientboundScreenFramePayload out = new ClientboundScreenFramePayload(
					sharer.getUUID(), payload.handle(), payload.streamId(), payload.frameSeq(),
					payload.width(), payload.height(), payload.chunkIndex(), payload.chunkCount(),
					plr.isScreenShareGrabAllowed(), payload.data());
			
			for(ServerPlayer player : sharer.server.getPlayerList().getPlayers()) {
				if(player == sharer) continue;
				ServerPlayNetworking.send(player, out);
			}
		});
		
		// Relay a stop notification to everyone so the shared display disappears.
		ServerPlayNetworking.registerGlobalReceiver(ServerboundStopSharePayload.TYPE, (payload, ctx) -> {
			ServerPlayer sharer = ctx.player();
			ClientboundStopSharePayload out = new ClientboundStopSharePayload(sharer.getUUID(), payload.streamId());
			for(ServerPlayer player : sharer.server.getPlayerList().getPlayers()) {
				if(player == sharer) continue;
				ServerPlayNetworking.send(player, out);
			}
		});
		
		// Store the client's sharing preferences on the server-side player so
		// relayed frames carry the right permissions.
		ServerPlayNetworking.registerGlobalReceiver(ServerboundShareSettingsPayload.TYPE, (payload, ctx) -> {
			((IMyServerPlayer) ctx.player()).setScreenShareGrabAllowed(payload.allowGrab());
		});
	}

	
}
