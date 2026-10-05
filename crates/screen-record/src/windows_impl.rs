//! DXGI monitor enumeration (Windows only).

use anyhow::{anyhow, Result};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_OUTPUT_DESC};

use crate::MonitorInfo;

pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>> {
    unsafe { enumerate_monitors_unsafe() }
}

unsafe fn enumerate_monitors_unsafe() -> Result<Vec<MonitorInfo>> {
    let factory: IDXGIFactory1 =
        CreateDXGIFactory1().map_err(|e| anyhow!("CreateDXGIFactory1 failed: {e}"))?;

    let mut monitors: Vec<MonitorInfo> = Vec::new();
    let mut next_id: usize = 0;

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
            let desc: DXGI_OUTPUT_DESC = output.GetDesc()?;

            if desc.AttachedToDesktop.as_bool() {
                let name = wide_to_string(&desc.DeviceName);
                let rect = desc.DesktopCoordinates;
                let w = (rect.right - rect.left).unsigned_abs();
                let h = (rect.bottom - rect.top).unsigned_abs();
                let display_name = if name.is_empty() {
                    format!("Monitor {}", next_id + 1)
                } else {
                    name
                };
                monitors.push(MonitorInfo {
                    id: next_id,
                    name: display_name,
                    x: rect.left,
                    y: rect.top,
                    width: w,
                    height: h,
                    primary: next_id == 0,
                });
                next_id += 1;
            }
            output_idx += 1;
        }
        adapter_idx += 1;
    }

    if monitors.is_empty() {
        anyhow::bail!("no DXGI monitors found");
    }
    Ok(monitors)
}

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}
