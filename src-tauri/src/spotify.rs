//! Spotify (etapa 4): Authorization Code + PKCE con redirect a http://127.0.0.1:PUERTO/callback.
//! El refresh token vive en el Administrador de Credenciales de Windows (crate `keyring`); el access token, solo en memoria.
//! Leer el álbum actual no requiere Premium; `play` sí.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
};

pub const REDIRECT_PORT: u16 = 47862;
const SCOPES: &str = "user-read-currently-playing user-read-playback-state user-modify-playback-state";
const SERVICE: &str = "jewelcase";
const ACCOUNT: &str = "spotify-refresh-token";
const API: &str = "https://api.spotify.com/v1";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";

struct Access {
    token: String,
    expires: Instant,
}
static ACCESS: Mutex<Option<Access>> = Mutex::const_new(None);

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())
}

fn refresh_token() -> Option<String> {
    entry().ok()?.get_password().ok()
}

pub fn is_connected() -> bool {
    refresh_token().is_some()
}

pub async fn logout() {
    if let Ok(e) = entry() {
        let _ = e.delete_credential();
    }
    *ACCESS.lock().await = None;
}

fn random_string(n: usize) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).expect("sin entropía del sistema");
    buf.iter().map(|b| CHARS[*b as usize % CHARS.len()] as char).collect()
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn redirect_uri() -> String {
    format!("http://127.0.0.1:{REDIRECT_PORT}/callback")
}

fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| {
            let mut out = Vec::new();
            let b = v.as_bytes();
            let mut i = 0;
            while i < b.len() {
                match b[i] {
                    b'%' if i + 2 < b.len() + 0 && i + 2 <= b.len() - 1 => {
                        out.push(u8::from_str_radix(&v[i + 1..i + 3], 16).unwrap_or(b'?'));
                        i += 3;
                    }
                    b'+' => {
                        out.push(b' ');
                        i += 1;
                    }
                    c => {
                        out.push(c);
                        i += 1;
                    }
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        })
    })
}

async fn token_request(client_id: &str, params: &[(&str, &str)]) -> Result<Value, String> {
    let mut form: Vec<(&str, &str)> = params.to_vec();
    form.push(("client_id", client_id));
    let c = reqwest::Client::new();
    let r = c.post(TOKEN_URL).form(&form).send().await.map_err(|e| e.to_string())?;
    let ok = r.status().is_success();
    let v: Value = r.json().await.map_err(|e| e.to_string())?;
    if !ok {
        return Err(format!("Spotify rechazó el token: {}", v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("error desconocido")));
    }
    Ok(v)
}

async fn store_tokens(v: &Value) -> Result<(), String> {
    let access = v["access_token"].as_str().ok_or("respuesta sin access_token")?.to_string();
    let secs = v["expires_in"].as_u64().unwrap_or(3600);
    *ACCESS.lock().await = Some(Access { token: access, expires: Instant::now() + Duration::from_secs(secs.saturating_sub(30)) });
    if let Some(r) = v["refresh_token"].as_str() {
        entry()?.set_password(r).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Abre el navegador, espera el redirect en 127.0.0.1 y canjea el código por tokens.
pub async fn login(client_id: &str) -> Result<(), String> {
    if client_id.trim().is_empty() {
        return Err("Falta el Client ID de Spotify".into());
    }
    let verifier = random_string(96);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_string(16);
    let listener = TcpListener::bind(("127.0.0.1", REDIRECT_PORT)).await.map_err(|e| format!("el puerto {REDIRECT_PORT} está ocupado: {e}"))?;
    let url = format!(
        "https://accounts.spotify.com/authorize?response_type=code&client_id={}&scope={}&redirect_uri={}&code_challenge_method=S256&code_challenge={}&state={}",
        enc(client_id.trim()),
        enc(SCOPES),
        enc(&redirect_uri()),
        challenge,
        state
    );
    open_browser(&url)?;

    let code = tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            let (mut sock, _) = listener.accept().await.map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.map_err(|e| e.to_string())?;
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let line = req.lines().next().unwrap_or("");
            let target = line.split_whitespace().nth(1).unwrap_or("");
            let Some(query) = target.strip_prefix("/callback?") else {
                let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                continue;
            };
            let (ok, msg) = if query_param(query, "state").as_deref() != Some(state.as_str()) {
                (false, "Estado inválido.".to_string())
            } else if let Some(e) = query_param(query, "error") {
                (false, format!("Spotify respondió: {e}"))
            } else if query_param(query, "code").is_some() {
                (true, "Listo, JewelCase quedó conectado a Spotify. Ya podés cerrar esta pestaña.".to_string())
            } else {
                (false, "Falta el código.".to_string())
            };
            let body = format!("<!doctype html><meta charset=utf-8><title>JewelCase</title><body style='font:16px system-ui;background:#0b0c0f;color:#e8ebf1;display:grid;place-items:center;height:100vh;margin:0'><p>{msg}</p>");
            let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.shutdown().await;
            return if ok { query_param(query, "code").ok_or_else(|| "sin código".to_string()) } else { Err(msg) };
        }
    })
    .await
    .map_err(|_| "Se agotó el tiempo esperando la autorización (3 minutos)".to_string())??;

    let v = token_request(client_id.trim(), &[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", &redirect_uri()), ("code_verifier", &verifier)]).await?;
    store_tokens(&v).await
}

async fn access_token(client_id: &str) -> Result<String, String> {
    {
        let a = ACCESS.lock().await;
        if let Some(a) = a.as_ref() {
            if a.expires > Instant::now() {
                return Ok(a.token.clone());
            }
        }
    }
    let rt = refresh_token().ok_or("No hay sesión de Spotify: conectate desde Ajustes")?;
    let v = token_request(client_id, &[("grant_type", "refresh_token"), ("refresh_token", &rt)]).await?;
    store_tokens(&v).await?;
    let a = ACCESS.lock().await;
    a.as_ref().map(|a| a.token.clone()).ok_or_else(|| "sin token".to_string())
}

/// Hasta cuándo Spotify nos pidió esperar (429). Mientras dure no se hace NINGÚN pedido: insistir alarga el bloqueo.
static COOLDOWN: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

fn cooldown_left() -> Option<u64> {
    let g = COOLDOWN.lock().ok()?;
    let t = (*g)?;
    let now = Instant::now();
    if t > now { Some((t - now).as_secs() + 1) } else { None }
}

/// Llama a la Web API. Devuelve None con 204 (nada sonando) y reintenta una vez si el token venció.
async fn api(client_id: &str, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Option<Value>, String> {
    if let Some(s) = cooldown_left() {
        return Err(format!("Spotify pidió esperar ({s} s)"));
    }
    let c = CLIENT.get_or_init(reqwest::Client::new);
    for attempt in 0..2 {
        let token = access_token(client_id).await?;
        let mut rb = c.request(method.clone(), format!("{API}{path}")).bearer_auth(token);
        if let Some(b) = &body {
            rb = rb.json(b);
        }
        let r = rb.send().await.map_err(|e| e.to_string())?;
        let st = r.status().as_u16();
        if st == 401 && attempt == 0 {
            *ACCESS.lock().await = None;
            continue;
        }
        if st == 204 {
            return Ok(None);
        }
        if st == 429 {
            let wait = r.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(30).clamp(5, 86_400);
            if let Ok(mut g) = COOLDOWN.lock() {
                *g = Some(Instant::now() + Duration::from_secs(wait));
            }
            return Err(format!("Spotify pidió esperar ({wait} s por demasiadas consultas)"));
        }
        let text = r.text().await.map_err(|e| e.to_string())?;
        if !(200..300).contains(&st) {
            let msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["error"]["message"].as_str().map(String::from)).unwrap_or(text);
            return Err(match st {
                403 => format!("Spotify: acción no permitida (¿hace falta Premium?): {msg}"),
                404 => format!("Spotify: no hay un dispositivo activo: {msg}"),
                _ => format!("Spotify {st}: {msg}"),
            });
        }
        return Ok(if text.is_empty() { None } else { serde_json::from_str(&text).ok() });
    }
    Err("no se pudo renovar la sesión de Spotify".into())
}

/// Qué suena ahora: pista y álbum, o `{ "playing": false }` si no hay nada.
pub async fn now(client_id: &str) -> Result<Value, String> {
    let Some(v) = api(client_id, reqwest::Method::GET, "/me/player/currently-playing", None).await? else {
        return Ok(json!({ "active": false }));
    };
    let item = &v["item"];
    if item["type"] != "track" {
        return Ok(json!({ "active": false }));
    }
    Ok(json!({
        "active": true,
        "is_playing": v["is_playing"].as_bool().unwrap_or(false),
        "progress_ms": v["progress_ms"],
        "track": { "id": item["id"], "name": item["name"], "uri": item["uri"], "number": item["track_number"], "disc": item["disc_number"], "duration_ms": item["duration_ms"] },
        "album": { "id": item["album"]["id"], "uri": item["album"]["uri"], "name": item["album"]["name"], "artist": item["artists"][0]["name"] },
    }))
}

/// Álbum completo: título, artista, año, UPC (si viene en `external_ids`) y lista de canciones.
pub async fn album(client_id: &str, id: &str) -> Result<Value, String> {
    let v = api(client_id, reqwest::Method::GET, &format!("/albums/{id}"), None).await?.ok_or("álbum vacío")?;
    let mut tracks: Vec<Value> = v["tracks"]["items"].as_array().cloned().unwrap_or_default();
    let total = v["tracks"]["total"].as_u64().unwrap_or(tracks.len() as u64) as usize;
    while tracks.len() < total {
        let more = api(client_id, reqwest::Method::GET, &format!("/albums/{id}/tracks?limit=50&offset={}", tracks.len()), None).await?;
        let items = more.as_ref().and_then(|m| m["items"].as_array()).cloned().unwrap_or_default();
        if items.is_empty() {
            break;
        }
        tracks.extend(items);
    }
    let fmt = |ms: u64| {
        let s = (ms + 500) / 1000;
        format!("{}:{:02}", s / 60, s % 60)
    };
    let list: Vec<Value> = tracks.iter().map(|t| json!({ "id": t["id"], "name": t["name"], "dur": fmt(t["duration_ms"].as_u64().unwrap_or(0)) })).collect();
    Ok(json!({
        "id": v["id"], "uri": v["uri"], "title": v["name"], "artist": v["artists"][0]["name"],
        "year": v["release_date"].as_str().and_then(|d| d.get(..4)).and_then(|y| y.parse::<i64>().ok()),
        "upc": v["external_ids"]["upc"], "cover": v["images"][0]["url"], "tracks": list,
    }))
}

/// Los próximos álbumes de la cola (distintos del actual, en orden, hasta `max`). `GET /me/player/queue` trae solo ~20 canciones: si al
/// álbum actual le quedan más que eso, el siguiente todavía no se ve. No requiere Premium.
pub async fn next_albums(client_id: &str, max: usize) -> Result<Value, String> {
    let Some(v) = api(client_id, reqwest::Method::GET, "/me/player/queue", None).await? else {
        return Ok(json!({ "albums": [] }));
    };
    let current = v["currently_playing"]["album"]["id"].as_str().map(String::from);
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<Value> = Vec::new();
    for t in v["queue"].as_array().into_iter().flatten() {
        if t["type"] != "track" {
            continue;
        }
        let Some(id) = t["album"]["id"].as_str() else { continue };
        if current.as_deref() == Some(id) || seen.iter().any(|s| s == id) {
            continue;
        }
        seen.push(id.to_string());
        out.push(json!({ "id": id, "name": t["album"]["name"], "artist": t["artists"][0]["name"] }));
        if out.len() >= max {
            break;
        }
    }
    Ok(json!({ "albums": out }))
}

/// Reproduce el álbum desde la canción `position` (requiere Premium y un dispositivo de Spotify activo).
pub async fn play(client_id: &str, album_uri: &str, position: u32) -> Result<(), String> {
    api(client_id, reqwest::Method::PUT, "/me/player/play", Some(json!({ "context_uri": album_uri, "offset": { "position": position } }))).await.map(|_| ())
}
