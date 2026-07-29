//! Windows Graphics Capture (WGC) — GPU-accurate per-window capture.
//!
//! GDI `PrintWindow` cannot read pixels that an app renders directly on the GPU
//! through DirectComposition / a DXGI swap-chain. Chromium/Electron apps
//! (Discord, Opera GX), and DirectX/Vulkan games do exactly that, so PrintWindow
//! returns a BLACK or stale frame for them — the "Discord is black / Opera
//! freezes" symptom.
//!
//! Windows Graphics Capture asks the DESKTOP COMPOSITOR (DWM) for the window's
//! real composed surface, so it captures whatever the window actually shows,
//! GPU-rendered content included. We create one capture session per window,
//! pull the latest frame as a Direct3D11 texture, copy it into a CPU-readable
//! staging texture, and read it out as BGRA — the same pixel format the rest of
//! the mod already expects.
//!
//! Each capture thread (one per window, see `capture.rs`) owns its own
//! `WgcCapture`, so a slow/stuck window never blocks another. If WGC cannot be
//! used for a window (old Windows, capture refused), the caller falls back to
//! the GDI `PrintWindow` path.

use windows::core::{Interface, IInspectable, Result as WinResult};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::{IDXGIAdapter, IDXGIDevice};

use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

/// Number of frames the WGC frame pool buffers. 2 is enough for steady polling
/// (one in flight, one being read) without adding latency.
const FRAME_POOL_BUFFERS: i32 = 2;

/// One live Windows Graphics Capture session bound to a single HWND. Owns the
/// D3D11 device used to read frames back to the CPU, the WGC frame pool and the
/// capture session. Dropping it stops the capture and releases everything.
pub struct WgcCapture {
    d3d_device: ID3D11Device,
    context: ID3D11DeviceContext,
    frame_pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    /// Last content size we sized the frame pool for; if the window resizes we
    /// recreate the pool at the new size.
    last_size: SizeInt32,
    /// Reusable CPU-readable staging texture, recreated only when the size
    /// changes — avoids allocating a GPU texture every frame.
    staging: Option<ID3D11Texture2D>,
    staging_w: i32,
    staging_h: i32,
}

// The WinRT/D3D11 COM objects are only ever touched from the single capture
// thread that owns this struct, so it is safe to move the struct to that
// thread. We assert Send so it can be created and stored per-thread.
unsafe impl Send for WgcCapture {}

impl WgcCapture {
    /// Create a capture session for `hwnd`. Returns None if WGC is unavailable
    /// or the window cannot be captured (caller falls back to PrintWindow).
    pub fn new(hwnd: HWND) -> Option<WgcCapture> {
        match Self::try_new(hwnd) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("[windowmod][wgc] capture init failed for {:?}: {e}", hwnd);
                None
            }
        }
    }

    fn try_new(hwnd: HWND) -> WinResult<WgcCapture> {
        unsafe {
            // 0) Initialise a COM/WinRT apartment on THIS capture thread.
            //    `IGraphicsCaptureItemInterop::CreateForWindow` is a WinRT call
            //    and requires the calling thread to be in a COM apartment;
            //    without this it fails with 0x80070057 (E_INVALIDARG) for every
            //    window — which is exactly why WGC "did nothing" and everything
            //    fell back to the GDI PrintWindow path (black Discord, frozen
            //    Opera). A multithreaded apartment is correct for our free-
            //    threaded frame pool. We ignore RPC_E_CHANGED_MODE in case the
            //    thread was already initialised in another mode.
            use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

            // 1) Create a D3D11 device (hardware) with BGRA support so the
            //    captured surface comes back in the BGRA byte order the mod uses.

            let mut d3d_device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                // No explicit adapter — let the driver pick the default GPU.
                // The turbofish disambiguates the generic IntoParam adapter
                // argument (E0283 otherwise).
                None::<&IDXGIAdapter>,
                D3D_DRIVER_TYPE_HARDWARE,
                // No software rasterizer module.
                windows::Win32::Foundation::HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d_device),
                None,
                Some(&mut context),
            )?;

            let d3d_device = d3d_device.ok_or_else(|| {
                windows::core::Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    "D3D11CreateDevice returned no device",
                )
            })?;
            let context = context.ok_or_else(|| {
                windows::core::Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    "D3D11CreateDevice returned no context",
                )
            })?;

            // 2) Wrap the D3D11 device as a WinRT IDirect3DDevice for WGC.
            let dxgi_device: IDXGIDevice = d3d_device.cast()?;
            let inspectable = CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device)?;
            let rt_device: IDirect3DDevice = inspectable.cast()?;

            // 3) Build a GraphicsCaptureItem for the target window via the
            //    interop factory (the WinRT API has no direct from-HWND ctor).
            let interop: IGraphicsCaptureItemInterop =
                windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem = interop.CreateForWindow(hwnd)?;

            let size = item.Size()?;

            // 4) Create the frame pool + capture session.
            let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &rt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                FRAME_POOL_BUFFERS,
                size,
            )?;
            let session = frame_pool.CreateCaptureSession(&item)?;

            // Try to disable the yellow capture border where supported (newer
            // Windows). Ignored on older builds.
            let _ = session.SetIsBorderRequired(false);
            // Try to disable the mouse-cursor capture (we don't want a cursor
            // baked into the window image). Ignored where unsupported.
            let _ = session.SetIsCursorCaptureEnabled(false);

            // A no-op closed handler keeps the item alive and avoids a warning;
            // we drive frames by polling TryGetNextFrame rather than events.
            let _ = item.Closed(&TypedEventHandler::<
                GraphicsCaptureItem,
                IInspectable,
            >::new(|_, _| Ok(())));

            session.StartCapture()?;

            Ok(WgcCapture {
                d3d_device,
                context,
                frame_pool,
                session,
                last_size: size,
                staging: None,
                staging_w: 0,
                staging_h: 0,
            })
        }
    }

    /// Grab the latest captured frame into `out` (BGRA, top-down, tightly
    /// packed). Returns Some((width,height)) on success, None when no new frame
    /// is ready yet (caller keeps the previous frame). The buffer is resized by
    /// the caller based on the returned size.
    ///
    /// `out` is a closure-free out-param: we return the dimensions and write the
    /// pixels into the provided growable Vec.
    pub fn grab(&mut self, out: &mut Vec<u8>) -> Option<(i32, i32)> {
        match self.try_grab(out) {
            Ok(dims) => dims,
            Err(e) => {
                eprintln!("[windowmod][wgc] grab failed: {e}");
                None
            }
        }
    }

    fn try_grab(&mut self, out: &mut Vec<u8>) -> WinResult<Option<(i32, i32)>> {
        unsafe {
            // Pull the most recent frame. TryGetNextFrame returns null when no
            // new frame is available since last call — that is not an error.
            let Ok(frame) = self.frame_pool.TryGetNextFrame() else {
                return Ok(None);
            };

            let content_size = frame.ContentSize()?;

            // If the window resized, recreate the frame pool at the new size so
            // subsequent frames are full-resolution.
            if content_size.Width != self.last_size.Width
                || content_size.Height != self.last_size.Height
            {
                let rt_device: IDirect3DDevice = {
                    let dxgi: IDXGIDevice = self.d3d_device.cast()?;
                    CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?
                };
                self.frame_pool.Recreate(
                    &rt_device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    FRAME_POOL_BUFFERS,
                    content_size,
                )?;
                self.last_size = content_size;
                // Drop this (now stale-sized) frame; next poll gets a fresh one.
                return Ok(None);
            }

            // Get the underlying D3D11 texture of this frame's surface.
            let surface = frame.Surface()?;
            let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
            let src_tex: ID3D11Texture2D = access.GetInterface()?;

            let mut desc = D3D11_TEXTURE2D_DESC::default();
            src_tex.GetDesc(&mut desc);
            let w = desc.Width as i32;
            let h = desc.Height as i32;
            if w < 1 || h < 1 {
                return Ok(None);
            }

            // Ensure a CPU-readable staging texture of the right size exists.
            if self.staging.is_none() || self.staging_w != w || self.staging_h != h {
                let staging_desc = D3D11_TEXTURE2D_DESC {
                    Width: desc.Width,
                    Height: desc.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: desc.Format,
                    SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut staging: Option<ID3D11Texture2D> = None;
                self.d3d_device
                    .CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
                self.staging = staging;
                self.staging_w = w;
                self.staging_h = h;
            }
            let staging = self.staging.as_ref().unwrap();

            // Copy the GPU frame into the staging texture, then map it for CPU
            // read-back.
            self.context.CopyResource(staging, &src_tex);

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;

            let row_pitch = mapped.RowPitch as usize;
            let dst_stride = (w as usize) * 4;
            let needed = dst_stride * (h as usize);
            if out.len() != needed {
                out.resize(needed, 0);
            }

            let src_base = mapped.pData as *const u8;
            // Copy row by row because the GPU row pitch may be larger than the
            // tight destination stride (alignment padding).
            for y in 0..h as usize {
                let src_row = src_base.add(y * row_pitch);
                let dst_row = out.as_mut_ptr().add(y * dst_stride);
                std::ptr::copy_nonoverlapping(src_row, dst_row, dst_stride);
            }

            self.context.Unmap(staging, 0);

            Ok(Some((w, h)))
        }
    }
}

impl Drop for WgcCapture {
    fn drop(&mut self) {
        // Stop the capture session cleanly. Errors here are not actionable.
        let _ = self.session.Close();
        let _ = self.frame_pool.Close();
    }
}
