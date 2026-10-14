//! Control de medios de Windows (SMTC): avisa al instante cuando Spotify cambia de canción o de estado (play/pausa), sin gastar ni un
//! pedido a la API de Spotify. Lo que se mira es local (el sistema), así que se puede revisar seguido sin costo.
//! Emite `smtc-changed` en cada cambio; `status()` dice si hay una sesión de Spotify visible (si no, el frontend consulta la API más seguido).
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use windows::{
    Media::Control::GlobalSystemMediaTransportControlsSessionManager as Manager,
    Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED},
};

static AVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn status() -> bool {
    AVAILABLE.load(Ordering::SeqCst)
}

/// Huella de la sesión de Spotify: estado + título + artista + álbum. `None` si no hay sesión de Spotify.
fn snapshot(mgr: &Manager) -> windows::core::Result<Option<String>> {
    for s in mgr.GetSessions()? {
        let id = s.SourceAppUserModelId()?.to_string().to_lowercase();
        if !id.contains("spotify") {
            continue;
        }
        let state = s.GetPlaybackInfo()?.PlaybackStatus()?.0;
        let p = s.TryGetMediaPropertiesAsync()?.join()?;
        return Ok(Some(format!("{state}|{}|{}|{}", p.Title()?, p.Artist()?, p.AlbumTitle()?)));
    }
    Ok(None)
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        // SAFETY: inicializa COM en este hilo, una sola vez.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut mgr: Option<Manager> = None;
        let mut last: Option<Option<String>> = None;
        loop {
            if mgr.is_none() {
                mgr = Manager::RequestAsync().and_then(|o| o.join()).ok();
            }
            match mgr.as_ref().map(snapshot) {
                Some(Ok(cur)) => {
                    if last.as_ref() != Some(&cur) {
                        AVAILABLE.store(cur.is_some(), Ordering::SeqCst);
                        let _ = app.emit("smtc-changed", json!({ "available": cur.is_some() }));
                        last = Some(cur);
                    }
                }
                _ => mgr = None, // error o sin administrador: se reintenta desde cero
            }
            std::thread::sleep(Duration::from_millis(700));
        }
    });
}
