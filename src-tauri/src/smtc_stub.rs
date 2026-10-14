//! SMTC es solo de Windows: en otras plataformas no hay sesión detectable y el frontend consulta la API de Spotify.
use tauri::AppHandle;

pub fn status() -> bool {
    false
}

pub fn spawn(_app: AppHandle) {}
