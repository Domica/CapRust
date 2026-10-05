//! Single-frame capture via DXGI Desktop Duplication.
//!
//! PR 2 scope: grab one BGRA frame from a monitor. The
//! streaming loop and ffmpeg pipe land in PR 3.

use anyhow::{anyhow, Result};
use windows::core::Interface as _;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput1, IDXGIResource, DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::Graphics::Gdi::HMONITOR;

/// One captured frame in BGRA8 order, tightly packed (no row
/// padding). `width * height * 4 == pixels.len()` always holds.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Capture exactly one frame from monitor `monitor_index` (0-based,
/// same order as `enumerate_monitors`). Returns a tightly packed
/// BGRA8 frame.
///
/// The Desktop Duplication API is exclusive: only one duplication
/// per output can exist at a time. This function creates and drops
/// the duplication before returning, so repeated calls are safe.
pub fn capture_one_frame(monitor_index: usize) -> Result<Frame> {
    unsafe { capture_one_frame_unsafe(monitor_index) }
}

unsafe fn capture_one_frame_unsafe(monitor_index: usize) -> Result<Frame> {
    // 1) D3D11 device with BGRA support so Desktop Duplication
    //    can hand us textures the CPU can read.
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let feature_levels = [D3D_FEATURE_LEVEL_11_0];
    D3D11CreateDevice(
        None,
        windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE,
        HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        Some(&feature_levels),
        D3D11_SDK_VERSION,
        Some(&mut device),
        None,
        Some(&mut context),
    )
    .map_err(|e| anyhow!("D3D11CreateDevice: {e}"))?;
    let device = device.ok_or_else(|| anyhow!("no D3D11 device"))?;
    let context = context.ok_or_else(|| anyhow!("no D3D11 immediate context"))?;

    // 2) Walk the same adapter/output ordering as enumerate_monitors
    //    and pick the one whose ordinal matches `monitor_index`.
    let factory: IDXGIFactory1 =
        CreateDXGIFactory1().map_err(|e| anyhow!("CreateDXGIFactory1: {e}"))?;
    let target_output = find_output_by_ordinal(&factory, monitor_index)?;

    // 3) DuplicateOutput needs IDXGIOutput1 and a device that was
    //    created with BGRA_SUPPORT.
    let output1: IDXGIOutput1 = target_output
        .cast()
        .map_err(|e| anyhow!("IDXGIOutput1 cast: {e}"))?;
    let duplication = output1
        .DuplicateOutput(&device)
        .map_err(|e| anyhow!("DuplicateOutput: {e}"))?;

    // 4) Warm-up: the first frame(s) right after DuplicateOutput are
    //    a black placeholder -- a documented DXGI quirk that every
    //    MS sample works around the same way. Acquire and release a
    //    few frames before taking the real one.
    for _ in 0..3 {
        let mut warm_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut warm_res: Option<IDXGIResource> = None;
        if duplication
            .AcquireNextFrame(200, &mut warm_info, &mut warm_res)
            .is_ok()
        {
            let _ = duplication.ReleaseFrame();
        }
    }

    // 5) Acquire the frame we actually use. 500 ms is generous: the
    //    first real frame after warm-up is usually ready in tens of
    //    ms, but a static desktop may wait for the next vsync.
    let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut resource: Option<IDXGIResource> = None;
    duplication
        .AcquireNextFrame(500, &mut frame_info, &mut resource)
        .map_err(|e| anyhow!("AcquireNextFrame: {e}"))?;
    let resource = resource.ok_or_else(|| anyhow!("no DXGI resource in frame"))?;

    let texture: ID3D11Texture2D = resource
        .cast()
        .map_err(|e| anyhow!("ID3D11Texture2D cast: {e}"))?;

    let mut desc = D3D11_TEXTURE2D_DESC::default();
    texture.GetDesc(&mut desc);
    let width = desc.Width;
    let height = desc.Height;

    // 5) Staging texture the CPU can Map. Same dimensions and format
    //    as the GPU texture, but USAGE_STAGING + CPU_ACCESS_READ.
    let mut staging_desc = desc;
    staging_desc.Usage = D3D11_USAGE_STAGING;
    staging_desc.BindFlags = 0;
    staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    staging_desc.MiscFlags = 0;
    let mut staging_opt: Option<ID3D11Texture2D> = None;
    device
        .CreateTexture2D(&staging_desc, None, Some(&mut staging_opt))
        .map_err(|e| anyhow!("CreateTexture2D(staging): {e}"))?;
    let staging = staging_opt.ok_or_else(|| anyhow!("no staging texture"))?;

    context.CopyResource(&staging, &texture);

    // 6) Map and copy into a tightly packed CPU buffer. RowPitch is
    //    usually width * 4 but the API makes no such promise, so we
    //    strip padding row by row.
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    context
        .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        .map_err(|e| anyhow!("Map(staging): {e}"))?;

    let row_pitch = mapped.RowPitch as usize;
    let row_bytes = (width as usize) * 4;
    let mut pixels = Vec::with_capacity(row_bytes * height as usize);
    let src = mapped.pData as *const u8;
    for y in 0..height as usize {
        let row_ptr = src.add(y * row_pitch);
        let slice = std::slice::from_raw_parts(row_ptr, row_bytes);
        pixels.extend_from_slice(slice);
    }

    context.Unmap(&staging, 0);
    duplication
        .ReleaseFrame()
        .map_err(|e| anyhow!("ReleaseFrame: {e}"))?;

    Ok(Frame {
        width,
        height,
        pixels,
    })
}

/// Find the Nth attached output across all adapters, matching the
/// ordering used by `enumerate_monitors`. Returns the IDXGIOutput
/// (NOT IDXGIOutput1 -- the caller casts).
unsafe fn find_output_by_ordinal(
    factory: &IDXGIFactory1,
    ordinal: usize,
) -> Result<windows::Win32::Graphics::Dxgi::IDXGIOutput> {
    let mut seen: usize = 0;
    let mut adapter_idx: u32 = 0;
    loop {
        let adapter = match factory.EnumAdapters1(adapter_idx) {
            Ok(a) => a,
            Err(_) => break,
        };
        let mut output_idx: u32 = 0;
        loop {
            let output = match adapter.EnumOutputs(output_idx) {
                Ok(o) => o,
                Err(_) => break,
            };
            let desc = output.GetDesc()?;
            if desc.AttachedToDesktop.as_bool() {
                if seen == ordinal {
                    return Ok(output);
                }
                seen += 1;
            }
            output_idx += 1;
        }
        adapter_idx += 1;
    }
    anyhow::bail!("monitor index {} out of range", ordinal);
}

// Silence the unused-import warning if HMONITOR is unused on some
// feature combination.
#[allow(dead_code)]
fn _keep_hmonitor_alive(_: HMONITOR) {}
