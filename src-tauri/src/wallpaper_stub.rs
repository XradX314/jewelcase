//! Wallpaper: solo implementado en Windows. En otras plataformas se informa un color sólido neutro y sin imagen.
use serde_json::{json, Value};

pub fn info() -> Value {
    json!({ "path": "", "style": 10, "tile": 0, "color": [32, 32, 32], "mtime": 0 })
}

pub fn bytes() -> Result<Vec<u8>, String> {
    Err("no hay wallpaper".into())
}
