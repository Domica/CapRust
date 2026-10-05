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
    DXGI_OUTDUPL_POINTER_SHAPE_INFO, DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR,
    DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME,
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
/// A live DXGI Desktop Duplication session. Holds the D3D11 device,
/// immediate context, and duplication for the lifetime of a recording
/// so the per-frame cost stays low. Drop releases everything.
pub struct DuplicationSession {
    context: ID3D11DeviceContext,
    device: ID3D11Device,
    duplication: windows::Win32::Graphics::Dxgi::IDXGIOutputDuplication,
    width: u32,
    height: u32,
}

impl DuplicationSession {
    /// Open a session on the Nth attached output (same ordering as
    /// `enumerate_monitors`). Does the DXGI warm-up internally.
    pub fn open(monitor_index: usize) -> Result<Self> {
        unsafe { Self::open_unsafe(monitor_index) }
    }

    /// Capture the next frame. Blocks up to `timeout_ms` waiting for
    /// a desktop change; on timeout returns `Ok(None)` so the caller
    /// can decide to repeat the last frame.
    pub fn next_frame(&self, timeout_ms: u32) -> Result<Option<Frame>> {
        unsafe {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut res: Option<IDXGIResource> = None;
            match self
                .duplication
                .AcquireNextFrame(timeout_ms, &mut info, &mut res)
            {
                Ok(()) => {}
                Err(e) => {
                    // WAIT_TIMEOUT is a normal outcome on a static
                    // desktop; anything else is a real error.
                    let code = e.code().0;
                    // DXGI_ERROR_WAIT_TIMEOUT = 0x887A0027
                    if code == 0x887A0027u32 as i32 {
                        return Ok(None);
                    }
                    anyhow::bail!("AcquireNextFrame: {e}");
                }
            }
            let Some(resource) = res else {
                let _ = self.duplication.ReleaseFrame();
                return Ok(None);
            };
            let texture: ID3D11Texture2D = resource
                .cast()
                .map_err(|e| anyhow!("ID3D11Texture2D cast: {e}"))?;

            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);

            let mut staging_desc = desc;
            staging_desc.Usage = D3D11_USAGE_STAGING;
            staging_desc.BindFlags = 0;
            staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            staging_desc.MiscFlags = 0;
            let mut staging_opt: Option<ID3D11Texture2D> = None;
            self.device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging_opt))
                .map_err(|e| anyhow!("CreateTexture2D(staging): {e}"))?;
            let staging = staging_opt.ok_or_else(|| anyhow!("no staging texture"))?;

            self.context.CopyResource(&staging, &texture);

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| anyhow!("Map(staging): {e}"))?;

            let row_pitch = mapped.RowPitch as usize;
            let row_bytes = (self.width as usize) * 4;
            let mut pixels = Vec::with_capacity(row_bytes * self.height as usize);
            let src = mapped.pData as *const u8;
            for y in 0..self.height as usize {
                let row_ptr = src.add(y * row_pitch);
                let slice = std::slice::from_raw_parts(row_ptr, row_bytes);
                pixels.extend_from_slice(slice);
            }
            self.context.Unmap(&staging, 0);

            // Cursor shape must be read BEFORE ReleaseFrame -- the
            // duplication invalidates it afterwards.
            let cursor = fetch_cursor(&self.duplication, &info).ok().flatten();

            self.duplication
                .ReleaseFrame()
                .map_err(|e| anyhow!("ReleaseFrame: {e}"))?;

            if let Some(c) = cursor {
                blend_cursor(&mut pixels, self.width, self.height, &c);
            }

            Ok(Some(Frame {
                width: self.width,
                height: self.height,
                pixels,
            }))
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }

    unsafe fn open_unsafe(monitor_index: usize) -> Result<Self> {
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

        let factory: IDXGIFactory1 =
            CreateDXGIFactory1().map_err(|e| anyhow!("CreateDXGIFactory1: {e}"))?;
        let target_output = find_output_by_ordinal(&factory, monitor_index)?;
        let output1: IDXGIOutput1 = target_output
            .cast()
            .map_err(|e| anyhow!("IDXGIOutput1 cast: {e}"))?;
        let duplication = output1
            .DuplicateOutput(&device)
            .map_err(|e| anyhow!("DuplicateOutput: {e}"))?;

        // Warm-up: first frames are a black placeholder. Release a few.
        for _ in 0..3 {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut res: Option<IDXGIResource> = None;
            if duplication
                .AcquireNextFrame(200, &mut info, &mut res)
                .is_ok()
            {
                let _ = duplication.ReleaseFrame();
            }
        }

        // Query the output dimensions from its DXGI output desc.
        let desc = target_output.GetDesc()?;
        let rect = desc.DesktopCoordinates;
        let width = (rect.right - rect.left).unsigned_abs();
        let height = (rect.bottom - rect.top).unsigned_abs();

        Ok(Self {
            context,
            device,
            duplication,
            width,
            height,
        })
    }
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

// ---------------------------------------------------------------------------
// Cursor overlay
// ---------------------------------------------------------------------------
//
// Desktop Duplication never bakes the cursor into the desktop image.
// Instead it hands us the pointer bitmap and position separately via
// GetFramePointerShape; the renderer composites it. Windows ships three
// shape formats: COLOR (32-bit BGRA + alpha), MONOCHROME (two 1-bit
// masks, AND + XOR), and MASKED_COLOR (rare, only older D3D9 apps).
// We handle the first two; MASKED_COLOR falls through as a no-op.

struct CursorShape {
    buffer: Vec<u8>,
    info: DXGI_OUTDUPL_POINTER_SHAPE_INFO,
    /// Hotspot-relative top-left in desktop coordinates.
    x: i32,
    y: i32,
}

unsafe fn fetch_cursor(
    duplication: &windows::Win32::Graphics::Dxgi::IDXGIOutputDuplication,
    info: &DXGI_OUTDUPL_FRAME_INFO,
) -> Result<Option<CursorShape>> {
    if !info.PointerPosition.Visible.as_bool() {
        return Ok(None);
    }
    let buf_size = info.PointerShapeBufferSize;
    if buf_size == 0 {
        return Ok(None);
    }
    let mut buffer = vec![0u8; buf_size as usize];
    let mut required: u32 = 0;
    let mut shape = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
    duplication
        .GetFramePointerShape(
            buf_size,
            buffer.as_mut_ptr() as *mut _,
            &mut required,
            &mut shape,
        )
        .map_err(|e| anyhow!("GetFramePointerShape: {e}"))?;

    // PointerPosition is the cursor hotspot in desktop coords; the
    // bitmap's top-left is hotspot-adjusted.
    let x = info.PointerPosition.Position.x - shape.HotSpot.x;
    let y = info.PointerPosition.Position.y - shape.HotSpot.y;

    Ok(Some(CursorShape {
        buffer,
        info: shape,
        x,
        y,
    }))
}

fn blend_cursor(frame: &mut [u8], fw: u32, fh: u32, c: &CursorShape) {
    match c.info.Type {
        t if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR.0 as u32 => {
            blend_cursor_color(frame, fw, fh, c)
        }
        t if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME.0 as u32 => {
            blend_cursor_mono(frame, fw, fh, c)
        }
        _ => {}
    }
}

fn blend_cursor_color(frame: &mut [u8], fw: u32, fh: u32, c: &CursorShape) {
    let cw = c.info.Width as i32;
    let ch = c.info.Height as i32;
    let pitch = c.info.Pitch as usize;
    for cy in 0..ch {
        let dy = c.y + cy;
        if dy < 0 || dy >= fh as i32 {
            continue;
        }
        for cx in 0..cw {
            let dx = c.x + cx;
            if dx < 0 || dx >= fw as i32 {
                continue;
            }
            let so = (cy as usize) * pitch + (cx as usize) * 4;
            if so + 4 > c.buffer.len() {
                continue;
            }
            let b = c.buffer[so] as u32;
            let g = c.buffer[so + 1] as u32;
            let r = c.buffer[so + 2] as u32;
            let a = c.buffer[so + 3] as u32;
            if a == 0 {
                continue;
            }
            let do_ = ((dy as u32 * fw + dx as u32) * 4) as usize;
            if do_ + 4 > frame.len() {
                continue;
            }
            if a == 255 {
                frame[do_] = b as u8;
                frame[do_ + 1] = g as u8;
                frame[do_ + 2] = r as u8;
                frame[do_ + 3] = 255;
            } else {
                let inv = 255 - a;
                let db = frame[do_] as u32;
                let dg = frame[do_ + 1] as u32;
                let dr = frame[do_ + 2] as u32;
                frame[do_] = ((b * a + db * inv) / 255) as u8;
                frame[do_ + 1] = ((g * a + dg * inv) / 255) as u8;
                frame[do_ + 2] = ((r * a + dr * inv) / 255) as u8;
                frame[do_ + 3] = 255;
            }
        }
    }
}

fn blend_cursor_mono(frame: &mut [u8], fw: u32, fh: u32, c: &CursorShape) {
    let cw = c.info.Width as i32;
    let ch = c.info.Height as i32;
    let pitch = c.info.Pitch as usize;
    let and_off = 0usize;
    let xor_off = pitch * ch as usize;
    for cy in 0..ch {
        let dy = c.y + cy;
        if dy < 0 || dy >= fh as i32 {
            continue;
        }
        for cx in 0..cw {
            let dx = c.x + cx;
            if dx < 0 || dx >= fw as i32 {
                continue;
            }
            let byte_x = (cx as usize) / 8;
            let bit = 7 - ((cx as usize) % 8);
            let row = (cy as usize) * pitch;
            let a_idx = and_off + row + byte_x;
            let x_idx = xor_off + row + byte_x;
            // GetFramePointerShape can return a buffer that is not
            // exactly and_off + pitch * ch (padding / tails), so bound
            // every read against the real buffer length.
            if a_idx >= c.buffer.len() || x_idx >= c.buffer.len() {
                continue;
            }
            let a_byte = c.buffer[a_idx];
            let x_byte = c.buffer[x_idx];
            let a_bit = (a_byte >> bit) & 1;
            let x_bit = (x_byte >> bit) & 1;
            let do_ = ((dy as u32 * fw + dx as u32) * 4) as usize;
            if do_ + 4 > frame.len() {
                continue;
            }
            match (a_bit, x_bit) {
                (1, 0) => {} // transparent
                (0, 0) => {
                    frame[do_] = 0;
                    frame[do_ + 1] = 0;
                    frame[do_ + 2] = 0;
                    frame[do_ + 3] = 255;
                }
                (0, 1) => {
                    frame[do_] = 255;
                    frame[do_ + 1] = 255;
                    frame[do_ + 2] = 255;
                    frame[do_ + 3] = 255;
                }
                (1, 1) => {
                    frame[do_] = 255 - frame[do_];
                    frame[do_ + 1] = 255 - frame[do_ + 1];
                    frame[do_ + 2] = 255 - frame[do_ + 2];
                    frame[do_ + 3] = 255;
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod cursor_blend_tests {
    use super::*;

    fn blank(w: u32, h: u32, b: u8, g: u8, r: u8) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            v.extend_from_slice(&[b, g, r, 255]);
        }
        v
    }

    #[test]
    fn color_cursor_full_alpha_overwrites() {
        let mut frame = blank(4, 4, 0, 0, 0);
        // 2x2 red cursor at (1,1)
        let mut buf = Vec::new();
        for _ in 0..4 {
            buf.extend_from_slice(&[0, 0, 255, 255]); // BGRA red
        }
        let shape = CursorShape {
            buffer: buf,
            info: DXGI_OUTDUPL_POINTER_SHAPE_INFO {
                Type: DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR.0 as u32,
                Width: 2,
                Height: 2,
                Pitch: 8,
                HotSpot: windows::Win32::Foundation::POINT { x: 0, y: 0 },
            },
            x: 1,
            y: 1,
        };
        blend_cursor(&mut frame, 4, 4, &shape);
        // pixel (1,1) and (2,1), (1,2), (2,2) must be red now
        for (px, py) in [(1u32, 1u32), (2, 1), (1, 2), (2, 2)] {
            let o = ((py * 4 + px) * 4) as usize;
            assert_eq!(frame[o], 0);
            assert_eq!(frame[o + 1], 0);
            assert_eq!(frame[o + 2], 255);
        }
        // pixel (0,0) untouched
        assert_eq!(frame[0], 0);
        assert_eq!(frame[1], 0);
        assert_eq!(frame[2], 0);
    }

    #[test]
    fn color_cursor_zero_alpha_skips() {
        let mut frame = blank(2, 2, 10, 10, 10);
        let buf = vec![0, 0, 255, 0]; // transparent red
        let shape = CursorShape {
            buffer: buf,
            info: DXGI_OUTDUPL_POINTER_SHAPE_INFO {
                Type: DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR.0 as u32,
                Width: 1,
                Height: 1,
                Pitch: 4,
                HotSpot: windows::Win32::Foundation::POINT { x: 0, y: 0 },
            },
            x: 0,
            y: 0,
        };
        blend_cursor(&mut frame, 2, 2, &shape);
        assert_eq!(frame[0], 10);
        assert_eq!(frame[1], 10);
        assert_eq!(frame[2], 10);
    }

    #[test]
    fn mono_cursor_white_opaque_black_transparent() {
        let mut frame = blank(2, 2, 100, 100, 100);
        // 2x2 monochrome: pixel (0,0) = white (and=0, xor=1),
        // (1,0) = transparent (and=1, xor=0), (0,1) = black (and=0, xor=0),
        // (1,1) = transparent.
        // AND mask row 0: bits 0b01000000 = 0x40 (px 0=0, px 1=1)
        // AND mask row 1: bits 0b01000000 = 0x40
        // XOR mask row 0: bits 0b10000000 = 0x80 (px 0=1, px 1=0)
        // XOR mask row 1: bits 0b00000000 = 0x00
        // Pitch is padded to 4-byte boundary: ceil(2/8)=1 byte -> pitch=4.
        let mut buf = vec![0u8; 16];
        // AND rows
        buf[0] = 0b0100_0000;
        buf[4] = 0b0100_0000;
        // XOR rows (offset 8)
        buf[8] = 0b1000_0000;
        buf[12] = 0;
        let shape = CursorShape {
            buffer: buf,
            info: DXGI_OUTDUPL_POINTER_SHAPE_INFO {
                Type: DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME.0 as u32,
                Width: 2,
                Height: 2,
                Pitch: 4,
                HotSpot: windows::Win32::Foundation::POINT { x: 0, y: 0 },
            },
            x: 0,
            y: 0,
        };
        blend_cursor(&mut frame, 2, 2, &shape);
        // (0,0) white
        assert_eq!(frame[0], 255);
        assert_eq!(frame[1], 255);
        assert_eq!(frame[2], 255);
        // (1,0) transparent -> unchanged 100
        assert_eq!(frame[4], 100);
        // (0,1) black
        assert_eq!(frame[8], 0);
        assert_eq!(frame[9], 0);
        assert_eq!(frame[10], 0);
        // (1,1) transparent
        assert_eq!(frame[12], 100);
    }

    #[test]
    fn cursor_offscreen_is_clipped() {
        let mut frame = blank(2, 2, 0, 0, 0);
        let mut buf = Vec::new();
        for _ in 0..4 {
            buf.extend_from_slice(&[255, 0, 0, 255]); // BGRA blue
        }
        let shape = CursorShape {
            buffer: buf,
            info: DXGI_OUTDUPL_POINTER_SHAPE_INFO {
                Type: DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR.0 as u32,
                Width: 2,
                Height: 2,
                Pitch: 8,
                HotSpot: windows::Win32::Foundation::POINT { x: 0, y: 0 },
            },
            x: 5,
            y: 5,
        };
        blend_cursor(&mut frame, 2, 2, &shape);
        // nothing changed
        for b in &frame {
            assert!(*b == 0 || *b == 255);
        }
        assert_eq!(frame[0], 0);
        assert_eq!(frame[2], 0);
    }
}
