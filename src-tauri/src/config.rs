//! Config en JSON dentro del directorio de configuración de la app. Los valores por defecto viven en
//! `default-config.json` (el frontend importa el mismo archivo), así hay una sola fuente.
use serde_json::Value;
use std::{fs, path::PathBuf, sync::Mutex};
use tauri::{AppHandle, Manager};

pub type Map = serde_json::Map<String, Value>;

const DEFAULTS: &str = include_str!("../default-config.json");
static CURRENT: Mutex<Option<Map>> = Mutex::new(None);

fn path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("config.json"))
}

fn defaults() -> Map {
    match serde_json::from_str::<Value>(DEFAULTS) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn load(app: &AppHandle) -> Map {
    let mut cfg = defaults();
    let saved = path(app).and_then(|p| fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str::<Value>(s.trim_start_matches('\u{feff}')).ok());
    if let Some(Value::Object(saved)) = saved {
        for (k, v) in saved {
            if cfg.contains_key(&k) {
                cfg.insert(k, v);
            }
        }
    }
    cfg
}

pub fn get(app: &AppHandle) -> Map {
    let mut cur = CURRENT.lock().unwrap();
    cur.get_or_insert_with(|| load(app)).clone()
}

/// Mezcla el parche (solo claves conocidas), guarda en disco y devuelve la config completa.
pub fn apply_patch(app: &AppHandle, patch: &Map) -> Map {
    let mut cur = CURRENT.lock().unwrap();
    let cfg = cur.get_or_insert_with(|| load(app));
    for (k, v) in patch {
        if cfg.contains_key(k) {
            cfg.insert(k.clone(), v.clone());
        }
    }
    if let Some(p) = path(app) {
        if let Some(dir) = p.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(&Value::Object(cfg.clone())) {
            let _ = fs::write(p, s);
        }
    }
    cfg.clone()
}
