//! Captura en vivo: solo existe en Windows. En otras plataformas no hace nada (el frontend usa el fondo estático).
use std::sync::atomic::AtomicBool;
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    AppHandle,
};

pub static BUSY: AtomicBool = AtomicBool::new(false);

pub fn start(_app: AppHandle, _channel: Channel<InvokeResponseBody>, _fps: u64) {}

pub fn stop(_app: &AppHandle) {}
