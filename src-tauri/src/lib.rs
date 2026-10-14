mod artwork;
#[cfg(windows)]
mod capture;
#[cfg(not(windows))]
#[path = "capture_stub.rs"]
mod capture;
mod config;
#[cfg(windows)]
mod smtc;
#[cfg(not(windows))]
#[path = "smtc_stub.rs"]
mod smtc;
mod spotify;
#[cfg(windows)]
mod wallpaper;
#[cfg(not(windows))]
#[path = "wallpaper_stub.rs"]
mod wallpaper;

use serde_json::{json, Value};
use std::time::Duration;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow, WindowEvent,
};

/// Lado base de la ventana (px lógicos) con "tamaño" = 100 %.
const BASE_SIZE: f64 = 400.0;

fn widget(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("widget")
}

/// Aplica a la ventana del widget lo que depende de la config: tamaño y nivel (encima / normal / al fondo).
fn apply_window_config(app: &AppHandle, cfg: &config::Map) {
    let Some(w) = widget(app) else { return };
    let size = cfg.get("size").and_then(Value::as_f64).unwrap_or(100.0);
    let side = (BASE_SIZE * size / 100.0).round();
    let _ = w.set_size(tauri::LogicalSize::new(side, side));
    let level = cfg.get("level").and_then(Value::as_str).unwrap_or("top");
    let _ = w.set_always_on_top(level == "top");
    let _ = w.set_always_on_bottom(level == "bottom");
}

/// Posición inicial: la guardada, o abajo a la derecha del monitor principal.
fn place_widget(app: &AppHandle, cfg: &config::Map) {
    let Some(w) = widget(app) else { return };
    let (x, y) = (cfg.get("x").and_then(Value::as_i64), cfg.get("y").and_then(Value::as_i64));
    if let (Some(x), Some(y)) = (x, y) {
        let _ = w.set_position(PhysicalPosition::new(x as i32, y as i32));
        return;
    }
    if let (Ok(Some(m)), Ok(s)) = (w.primary_monitor(), w.outer_size()) {
        let mp = m.position();
        let ms = m.size();
        let _ = w.set_position(PhysicalPosition::new(
            mp.x + ms.width as i32 - s.width as i32 - 60,
            mp.y + ms.height as i32 - s.height as i32 - 100,
        ));
    }
}

/// Monitor donde está el widget y área cliente de la ventana, en píxeles físicos.
fn layout(w: &WebviewWindow) -> Option<Value> {
    let p = w.inner_position().ok()?;
    let s = w.inner_size().ok()?;
    let m = w.current_monitor().ok()??;
    Some(json!({
        "mon": { "x": m.position().x, "y": m.position().y, "w": m.size().width, "h": m.size().height },
        "win": { "x": p.x, "y": p.y, "w": s.width, "h": s.height },
    }))
}

#[tauri::command]
fn get_config(app: AppHandle) -> Value {
    Value::Object(config::get(&app))
}

#[tauri::command]
fn set_config(app: AppHandle, patch: config::Map) {
    let cfg = config::apply_patch(&app, &patch);
    // la posición la manda la propia ventana al moverse: no se vuelve a aplicar (evita un ciclo)
    if patch.contains_key("size") || patch.contains_key("level") {
        apply_window_config(&app, &cfg);
    }
    let _ = app.emit("config-changed", Value::Object(cfg));
}

#[tauri::command]
fn layout_info(app: AppHandle) -> Option<Value> {
    widget(&app).and_then(|w| layout(&w))
}

#[tauri::command]
fn wallpaper_info() -> Value {
    wallpaper::info()
}

#[tauri::command]
fn wallpaper_bytes() -> Result<tauri::ipc::Response, String> {
    wallpaper::bytes().map(tauri::ipc::Response::new)
}

/// Empieza a mandar frames de lo que hay detrás del widget (ver capture.rs).
#[tauri::command]
fn start_capture(app: AppHandle, on_frame: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>, fps: Option<u64>) {
    capture::start(app, on_frame, fps.unwrap_or(30));
}

#[tauri::command]
fn stop_capture(app: AppHandle) {
    capture::stop(&app);
}

/// Busca el álbum (MusicBrainz + Cover Art Archive), procesa la foto del CD y deja todo en el caché.
#[tauri::command]
async fn resolve_album(app: AppHandle, artist: String, album: String, upc: Option<String>, force: bool, want_tracks: Option<bool>, need_front: Option<bool>) -> Result<Value, String> {
    artwork::resolve(&app, &artist, &album, upc.as_deref(), force, want_tracks.unwrap_or(true), need_front.unwrap_or(true)).await
}

/// Fija una foto del CD elegida a mano (URL) para un álbum.
#[tauri::command]
async fn set_disc_override(app: AppHandle, artist: String, album: String, url: String) -> Result<Value, String> {
    artwork::set_override(&app, &artist, &album, &url).await
}

#[tauri::command]
fn clear_disc_override(app: AppHandle, artist: String, album: String) -> Result<(), String> {
    artwork::clear_override(&app, &artist, &album)
}

#[tauri::command]
fn read_cache_file(app: AppHandle, name: String) -> Result<tauri::ipc::Response, String> {
    artwork::read_cache(&app, &name).map(tauri::ipc::Response::new)
}

#[tauri::command]
fn smtc_status() -> bool {
    smtc::status()
}

#[tauri::command]
fn spotify_status() -> bool {
    spotify::is_connected()
}

#[tauri::command]
async fn spotify_login(app: AppHandle, client_id: String) -> Result<(), String> {
    spotify::login(&client_id).await?;
    let _ = app.emit("spotify-changed", ());
    Ok(())
}

#[tauri::command]
async fn spotify_logout(app: AppHandle) {
    spotify::logout().await;
    let _ = app.emit("spotify-changed", ());
}

#[tauri::command]
async fn spotify_now(client_id: String) -> Result<Value, String> {
    spotify::now(&client_id).await
}

#[tauri::command]
async fn spotify_album(client_id: String, id: String) -> Result<Value, String> {
    spotify::album(&client_id, &id).await
}

#[tauri::command]
async fn spotify_queue(client_id: String) -> Result<Value, String> {
    spotify::next_albums(&client_id, 2).await
}

#[tauri::command]
async fn spotify_play(client_id: String, album_uri: String, position: u32) -> Result<(), String> {
    spotify::play(&client_id, &album_uri, position).await
}

fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let settings = MenuItem::with_id(app, "settings", "Ajustes", true, None::<&str>)?;
    let mover = MenuItem::with_id(app, "move", "Reposicionar", true, None::<&str>)?;
    let reload = MenuItem::with_id(app, "reload", "Recargar arte", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Salir", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings, &mover, &reload, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("JewelCase")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "settings" => show_settings(app),
            "move" => {
                let _ = app.emit("toggle-move", ());
            }
            "reload" => {
                let _ = app.emit("reload-art", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_settings(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Posición global del cursor a ~30 Hz, relativa a la ventana del widget (px físicos).
/// El frontend hace el raycast contra la caja y activa o desactiva el click-through.
fn spawn_cursor_poller(app: AppHandle) {
    std::thread::spawn(move || {
        let mut was_inside = false;
        loop {
            std::thread::sleep(Duration::from_millis(33));
            let Some(w) = widget(&app) else { continue };
            if !w.is_visible().unwrap_or(false) {
                continue;
            }
            let (Ok(c), Ok(p), Ok(s)) = (app.cursor_position(), w.inner_position(), w.inner_size()) else { continue };
            let x = c.x - p.x as f64;
            let y = c.y - p.y as f64;
            let inside = x >= 0.0 && y >= 0.0 && x < s.width as f64 && y < s.height as f64;
            if inside || was_inside {
                // al salir manda un último evento (fuera de la ventana) para volver al click-through
                let _ = app.emit_to("widget", "cursor", json!({ "x": x, "y": y }));
            }
            was_inside = inside;
        }
    });
}

#[cfg(windows)]
#[link(name = "shell32")]
extern "system" {
    fn SHQueryUserNotificationState(state: *mut i32) -> i32;
}

/// 1 = pantalla bloqueada / protector, 2 = app a pantalla completa, 3 = juego Direct3D, 4 = presentación. Fuera de Windows no se detecta.
#[cfg(windows)]
fn notification_state() -> Option<i32> {
    let mut state: i32 = 5;
    // SAFETY: el puntero apunta a un entero local válido.
    let ok = unsafe { SHQueryUserNotificationState(&mut state) } >= 0;
    ok.then_some(state)
}

#[cfg(not(windows))]
fn notification_state() -> Option<i32> {
    None
}

/// Cada ~1,5 s mira si hay una app a pantalla completa (juego, video, presentación) o la pantalla está bloqueada. Si es así, el widget
/// no dibuja ni captura nada hasta que termine: no tiene sentido gastar GPU detrás de un juego.
fn spawn_busy_poller(app: AppHandle) {
    std::thread::spawn(move || {
        let mut was = false;
        loop {
            std::thread::sleep(Duration::from_millis(1500));
            let enabled = config::get(&app).get("pauseFullscreen").and_then(Value::as_bool).unwrap_or(true);
            let busy = enabled && matches!(notification_state(), Some(1 | 2 | 3 | 4));
            if busy != was {
                was = busy;
                capture::BUSY.store(busy, std::sync::atomic::Ordering::SeqCst);
                let _ = app.emit("app-busy", busy);
            }
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_config,
            set_config,
            layout_info,
            wallpaper_info,
            wallpaper_bytes,
            start_capture,
            stop_capture,
            resolve_album,
            read_cache_file,
            set_disc_override,
            clear_disc_override,
            smtc_status,
            spotify_status,
            spotify_login,
            spotify_logout,
            spotify_now,
            spotify_queue,
            spotify_album,
            spotify_play
        ])
        .on_window_event(|window, event| match (window.label(), event) {
            // la ventana de ajustes se minimiza en vez de cerrarse: su botón queda en la barra de tareas
            ("settings", WindowEvent::CloseRequested { api, .. }) => {
                api.prevent_close();
                let _ = window.minimize();
            }
            ("widget", WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }) => {
                if let Some(w) = window.app_handle().get_webview_window("widget") {
                    if let Some(l) = layout(&w) {
                        let _ = window.app_handle().emit("widget-layout", l);
                    }
                }
            }
            _ => {}
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let cfg = config::get(&handle);
            apply_window_config(&handle, &cfg);
            place_widget(&handle, &cfg);
            if let Some(w) = widget(&handle) {
                let _ = w.show();
            }
            // el botón de Ajustes vive siempre en la barra de tareas (arranca minimizado)
            if let Some(w) = handle.get_webview_window("settings") {
                let _ = w.show();
                let _ = w.minimize();
            }
            build_tray(&handle)?;
            spawn_busy_poller(handle.clone());
            smtc::spawn(handle.clone());
            spawn_cursor_poller(handle);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error al iniciar JewelCase");
}
