//! Wallpaper real de Windows: ruta (SPI_GETDESKWALLPAPER), estilo de ajuste y color de fondo (registro).
use serde_json::{json, Value};
use std::{ffi::c_void, fs, path::PathBuf, time::UNIX_EPOCH};
use winreg::{enums::HKEY_CURRENT_USER, RegKey};

const SPI_GETDESKWALLPAPER: u32 = 0x0073;

#[link(name = "user32")]
extern "system" {
    fn SystemParametersInfoW(action: u32, param: u32, pv_param: *mut c_void, win_ini: u32) -> i32;
}

fn wallpaper_path() -> Option<PathBuf> {
    let mut buf = [0u16; 520];
    // SAFETY: el buffer tiene espacio para MAX_PATH wchar y se pasa su tamaño en uiParam.
    let ok = unsafe { SystemParametersInfoW(SPI_GETDESKWALLPAPER, buf.len() as u32, buf.as_mut_ptr() as *mut c_void, 0) };
    if ok == 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    if len == 0 {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

fn desktop_string(key: &str, name: &str) -> Option<String> {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(key).ok()?.get_value::<String, _>(name).ok()
}

/// Ruta, estilo (10 rellenar, 6 ajustar, 2 estirar, 22 extender, 0 centrar/mosaico), mosaico, color sólido y mtime.
pub fn info() -> Value {
    let path = wallpaper_path().filter(|p| p.is_file());
    let style = desktop_string(r"Control Panel\Desktop", "WallpaperStyle").and_then(|s| s.trim().parse::<i64>().ok()).unwrap_or(10);
    let tile = desktop_string(r"Control Panel\Desktop", "TileWallpaper").and_then(|s| s.trim().parse::<i64>().ok()).unwrap_or(0);
    let color: Vec<u8> = desktop_string(r"Control Panel\Colors", "Background")
        .map(|s| s.split_whitespace().filter_map(|n| n.parse::<u8>().ok()).collect())
        .filter(|v: &Vec<u8>| v.len() == 3)
        .unwrap_or_else(|| vec![0, 0, 0]);
    let mtime = path
        .as_ref()
        .and_then(|p| fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    json!({
        "path": path.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        "style": style, "tile": tile, "color": color, "mtime": mtime,
    })
}

pub fn bytes() -> Result<Vec<u8>, String> {
    let p = wallpaper_path().ok_or("no hay wallpaper")?;
    fs::read(&p).map_err(|e| format!("no se pudo leer {}: {e}", p.display()))
}
