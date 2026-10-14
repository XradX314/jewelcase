//! Resolvedor de imágenes reales (puerto de prototype/resolver.py): MusicBrainz → Cover Art Archive → otros proveedores.
//!
//! Orden de trabajo, pensado para tardar lo menos posible:
//! 1. Con el UPC de Spotify se identifica la edición exacta (1 consulta a MusicBrainz) y se miran sus imágenes en Cover Art Archive,
//!    que no tiene límite de consultas. Solo si ahí no hay foto de disco se listan las demás ediciones y, después, se busca por nombre.
//! 2. Las búsquedas por nombre en TheAudioDB y Discogs arrancan al principio, en paralelo con MusicBrainz.
//! 3. Las descargas y el procesamiento de las imágenes candidatas se hacen en paralelo.
//! 4. La lista de canciones de MusicBrainz solo se pide si Spotify no la trajo.
//!
//! Procesa la foto del disco (`disc_crop`) y guarda todo en el caché de la app. Devuelve rutas relativas al caché (ver `read_cache_file`).
mod circle;
mod disc_crop;

use futures::future::join_all;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    sync::{mpsc, oneshot, watch, Mutex, Notify},
    task::{JoinHandle, JoinSet},
};

const UA: &str = "JewelCaseWidget/0.1 (uso personal)";
const MB: &str = "https://musicbrainz.org/ws/2";
const CAA: &str = "https://coverartarchive.org";
const MAX_RELEASES: usize = 20;
/// Sube cuando cambia la forma de resolver (p. ej. un proveedor nuevo), para no quedarse con resultados viejos en el caché.
const CACHE_VERSION: i64 = 6;
/// Un resultado SIN disco caduca: las fuentes y la lógica mejoran, y un "no hay" viejo no tiene por qué seguir valiendo.
const NEGATIVE_TTL_SECS: u64 = 6 * 3600;

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

macro_rules! log {
    ($($t:tt)*) => { eprintln!("[arte] {}", format!($($t)*)) };
}

/// MusicBrainz pide como máximo 1 consulta por segundo.
static LAST_MB: Mutex<Option<Instant>> = Mutex::const_new(None);
static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Un solo cliente para todo el programa: reutiliza conexiones (y el handshake TLS) entre consultas.
fn client() -> Result<reqwest::Client, String> {
    if let Some(c) = CLIENT.get() {
        return Ok(c.clone());
    }
    let c = reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(25))
        .connect_timeout(Duration::from_secs(8))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(CLIENT.get_or_init(|| c).clone())
}

/// El mensaje de reqwest ("error sending request") esconde la causa real; esto arma toda la cadena.
fn chain(e: &dyn Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        s.push_str(" <- ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

async fn get_json(c: &reqwest::Client, url: &str, mb: bool) -> Result<Option<Value>, String> {
    get_json_h(c, url, mb, None).await
}

async fn get_json_h(c: &reqwest::Client, url: &str, mb: bool, auth: Option<&str>) -> Result<Option<Value>, String> {
    // MusicBrainz responde 503 cuando está saturado: ahí conviene esperar más; en los demás, un reintento rápido alcanza
    let base_ms: u64 = if mb { 1500 } else { 500 };
    // los servicios que no son MusicBrainz (Cover Art Archive, etc.) a veces tardan ~10 s en devolver un 500: no vale insistir tanto
    let max_attempts: u64 = if mb { 5 } else { 3 };
    for attempt in 0..max_attempts {
        if mb {
            let mut last = LAST_MB.lock().await;
            if let Some(t) = *last {
                let wait = Duration::from_millis(1100).saturating_sub(t.elapsed());
                if !wait.is_zero() {
                    tokio::time::sleep(wait).await;
                }
            }
            *last = Some(Instant::now());
        }
        let mut rb = c.get(url).header("Accept", "application/json");
        if !mb {
            rb = rb.timeout(Duration::from_secs(10));
        }
        if let Some(a) = auth {
            rb = rb.header("Authorization", a);
        }
        let resp = match rb.send().await {
            Ok(r) => r,
            Err(e) if attempt + 1 < max_attempts => {
                log!("reintento {} de {url}: {}", attempt + 1, chain(&e));
                tokio::time::sleep(Duration::from_millis(700 * (attempt + 1))).await;
                continue;
            }
            Err(e) => return Err(format!("sin conexión con {url}: {}", chain(&e))),
        };
        let st = resp.status();
        if st.as_u16() == 404 {
            return Ok(None);
        }
        if matches!(st.as_u16(), 429 | 500 | 502 | 503) && attempt + 1 < max_attempts {
            log!("{st} en {url}, reintento");
            tokio::time::sleep(Duration::from_millis(base_ms * (attempt + 1))).await;
            continue;
        }
        let resp = resp.error_for_status().map_err(|e| chain(&e))?;
        return resp.json::<Value>().await.map(Some).map_err(|e| chain(&e));
    }
    Err("demasiados reintentos".into())
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.replace('"', "").bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(Clone)]
struct Group {
    id: String,
    title: String,
    artist: String,
    year: Option<i64>,
    exact_release: Option<String>,
}

/// La edición exacta del UPC de Spotify (y su release group).
async fn barcode_release(c: &reqwest::Client, upc: &str, artist: &str, album: &str) -> Result<Option<Group>, String> {
    let d = get_json(c, &format!("{MB}/release/?query=barcode:{}&fmt=json&limit=5", enc(upc)), true).await?;
    let Some(r) = d.as_ref().and_then(|d| d["releases"].as_array()).and_then(|a| a.first()) else { return Ok(None) };
    let Some(id) = r["release-group"]["id"].as_str() else { return Ok(None) };
    Ok(Some(Group {
        id: id.into(),
        title: r["title"].as_str().unwrap_or(album).into(),
        artist: r["artist-credit"][0]["name"].as_str().unwrap_or(artist).into(),
        year: r["date"].as_str().and_then(|s| s.get(..4)).and_then(|y| y.parse().ok()),
        exact_release: r["id"].as_str().map(String::from),
    }))
}

/// Release groups que coinciden por nombre: primero los de tipo Album, hasta 4 (a veces el mismo álbum está duplicado en
/// MusicBrainz y la foto del disco está en otro grupo).
async fn name_groups(c: &reqwest::Client, artist: &str, album: &str) -> Result<Vec<Group>, String> {
    let query = enc(&format!("releasegroup:\"{album}\" AND artist:\"{artist}\""));
    let d = get_json(c, &format!("{MB}/release-group/?query={query}&fmt=json&limit=5"), true).await?;
    let groups: Vec<&Value> = d
        .as_ref()
        .and_then(|d| d["release-groups"].as_array())
        .map(|a| a.iter().filter(|g| g["score"].as_i64().unwrap_or(0) >= 85).collect())
        .unwrap_or_default();
    let mut ordered: Vec<&Value> = groups.iter().copied().filter(|g| g["primary-type"] == "Album").collect();
    ordered.extend(groups.iter().copied().filter(|g| g["primary-type"] != "Album"));
    Ok(ordered
        .into_iter()
        .take(4)
        .filter_map(|g| {
            Some(Group {
                id: g["id"].as_str()?.into(),
                title: g["title"].as_str().unwrap_or(album).into(),
                artist: g["artist-credit"][0]["name"].as_str().unwrap_or(artist).into(),
                year: g["first-release-date"].as_str().and_then(|s| s.get(..4)).and_then(|y| y.parse().ok()),
                exact_release: None,
            })
        })
        .collect())
}

struct CdRelease {
    id: String,
    art_count: i64,
}

/// Ediciones en CD del release group: las que tienen imágenes (las más prometedoras primero) y la primera en CD con o sin imágenes.
async fn cd_releases(c: &reqwest::Client, rg: &str) -> Result<(Vec<CdRelease>, Option<String>), String> {
    let (mut with_art, mut first_cd, mut offset) = (Vec::new(), None, 0usize);
    loop {
        let url = format!("{MB}/release?release-group={rg}&inc=media&fmt=json&limit=100&offset={offset}");
        let Some(d) = get_json(c, &url, true).await? else { break };
        let rels = d["releases"].as_array().cloned().unwrap_or_default();
        for r in &rels {
            let is_cd = r["media"].as_array().is_some_and(|m| m.iter().any(|x| x["format"].as_str().is_some_and(|f| f.contains("CD"))));
            if !is_cd {
                continue;
            }
            let id = r["id"].as_str().unwrap_or_default().to_string();
            if first_cd.is_none() {
                first_cd = Some(id.clone());
            }
            if r["cover-art-archive"]["artwork"].as_bool().unwrap_or(false) {
                with_art.push(CdRelease { id, art_count: r["cover-art-archive"]["count"].as_i64().unwrap_or(0) });
            }
        }
        offset += rels.len();
        if rels.is_empty() || offset as i64 >= d["release-count"].as_i64().unwrap_or(0) {
            break;
        }
    }
    with_art.sort_by_key(|r| -r.art_count); // más imágenes = más chances de tener foto del disco
    Ok((with_art, first_cd))
}

/// Todas las imágenes de una edición, por tipo (Front, Medium...), en UN solo pedido. Prefiere la miniatura de 1200 px.
async fn release_images(c: &reqwest::Client, release: &str) -> Result<HashMap<String, Vec<String>>, String> {
    let d = get_json(c, &format!("{CAA}/release/{release}"), false).await?;
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for im in d.as_ref().and_then(|d| d["images"].as_array()).into_iter().flatten() {
        let th = &im["thumbnails"];
        let Some(u) = th["1200"].as_str().or(th["large"].as_str()).or(im["image"].as_str()) else { continue };
        for t in im["types"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            out.entry(t.to_string()).or_default().push(u.replace("http://", "https://"));
        }
    }
    Ok(out)
}

async fn download(c: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    download_with(c, url, None).await
}

async fn download_with(c: &reqwest::Client, url: &str, auth: Option<&str>) -> Result<Vec<u8>, String> {
    let mut last = String::new();
    for attempt in 0..3u64 {
        let mut rb = c.get(url).timeout(Duration::from_secs(20));
        if let Some(a) = auth {
            rb = rb.header("Authorization", a);
        }
        match rb.send().await.and_then(|r| r.error_for_status()) {
            Ok(r) => match r.bytes().await {
                Ok(b) => return Ok(b.to_vec()),
                Err(e) => last = chain(&e),
            },
            Err(e) => last = chain(&e),
        }
        log!("descarga fallida ({url}): {last}");
        tokio::time::sleep(Duration::from_millis(500 * (attempt + 1))).await;
    }
    Err(last)
}

/// Lista de canciones de una edición: `release/{id}?inc=recordings`.
async fn tracklist(c: &reqwest::Client, release: &str) -> Result<Vec<(String, String)>, String> {
    let d = get_json(c, &format!("{MB}/release/{release}?inc=recordings&fmt=json"), true).await?;
    let mut out = Vec::new();
    for m in d.as_ref().and_then(|d| d["media"].as_array()).into_iter().flatten() {
        for t in m["tracks"].as_array().into_iter().flatten() {
            let secs = t["length"].as_u64().map(|ms| (ms + 500) / 1000).unwrap_or(0);
            out.push((t["title"].as_str().unwrap_or("?").to_string(), format!("{}:{:02}", secs / 60, secs % 60)));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------------------------
// Proveedores alternativos de discos
// ---------------------------------------------------------------------------------------------------------------------------

/// Lo que devuelve un proveedor: URLs candidatas (con el proveedor), notas para el informe y si algo falló por red.
#[derive(Default)]
struct Found {
    urls: Vec<(&'static str, String)>,
    notes: Vec<String>,
    error: bool,
}

impl Found {
    fn add(&mut self, prov: &'static str, u: &str) {
        let u = u.replace("http://", "https://");
        if !self.urls.iter().any(|(_, x)| *x == u) {
            self.urls.push((prov, u));
        }
    }
    fn merge(&mut self, o: Found) {
        for (p, u) in o.urls {
            self.add(p, &u);
        }
        self.notes.extend(o.notes);
        self.error |= o.error;
    }
    fn count(&self, prov: &str) -> usize {
        self.urls.iter().filter(|(p, _)| *p == prov).count()
    }
}

fn audiodb_urls(f: &mut Found, d: &Value) {
    for a in d["album"].as_array().into_iter().flatten() {
        if let Some(u) = a["strAlbumCDart"].as_str().filter(|u| !u.is_empty()) {
            f.add("TheAudioDB", u);
        }
    }
}

/// TheAudioDB por nombre (clave pública de prueba "2").
async fn audiodb_by_name(c: &reqwest::Client, artist: &str, album: &str) -> Found {
    let mut f = Found::default();
    match get_json(c, &format!("https://www.theaudiodb.com/api/v1/json/2/searchalbum.php?s={}&a={}", enc(artist), enc(album)), false).await {
        Ok(Some(d)) => audiodb_urls(&mut f, &d),
        Ok(None) => {}
        Err(e) => {
            log!("TheAudioDB (búsqueda): {e}");
            f.error = true;
        }
    }
    f
}

/// TheAudioDB por release group de MusicBrainz, todos los grupos a la vez.
async fn audiodb_by_mbid(c: &reqwest::Client, groups: &[String]) -> Found {
    let rs = join_all(groups.iter().map(|g| async move { (g, get_json(c, &format!("https://www.theaudiodb.com/api/v1/json/2/album-mb.php?i={g}"), false).await) })).await;
    let mut f = Found::default();
    for (g, r) in rs {
        match r {
            Ok(Some(d)) => audiodb_urls(&mut f, &d),
            Ok(None) => {}
            Err(e) => {
                log!("TheAudioDB {g}: {e}");
                f.error = true;
            }
        }
    }
    f
}

/// Fanart.tv ("cdart", hace falta una API key gratuita), todos los grupos a la vez.
async fn fanart(c: &reqwest::Client, groups: &[String], key: &str) -> Found {
    let rs = join_all(groups.iter().map(|g| async move { (g, get_json(c, &format!("https://webservice.fanart.tv/v3/music/albums/{g}?api_key={}", enc(key)), false).await) })).await;
    let mut f = Found::default();
    for (g, r) in rs {
        match r {
            Ok(Some(d)) => {
                let mut arts: Vec<&Value> = d["albums"][g.as_str()]["cdart"].as_array().into_iter().flatten().collect();
                arts.sort_by_key(|a| -a["likes"].as_str().and_then(|l| l.parse::<i64>().ok()).unwrap_or(0));
                for a in arts {
                    if let Some(u) = a["url"].as_str() {
                        f.add("Fanart.tv", u);
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                log!("Fanart.tv: {e}");
                f.error = true;
            }
        }
    }
    f
}

/// Discogs (hace falta un token personal gratuito): busca ediciones en CD por artista y título y devuelve las imágenes de cada una,
/// sin la primaria (que es la tapa). Discogs no etiqueta las fotos de disco: que sean un CD y del lado impreso lo valida `disc_crop`
/// (modo estricto). Los escaneos de disco no siempre son cuadrados (p. ej. 600×537): se acepta de 0,8 a 1,25 de proporción.
async fn discogs(c: &reqwest::Client, artist: &str, album: &str, auth: &str) -> Found {
    let mut f = Found::default();
    let url = format!("https://api.discogs.com/database/search?type=release&format=CD&artist={}&release_title={}&per_page=6", enc(artist), enc(album));
    let d = match get_json_h(c, &url, false, Some(auth)).await {
        Ok(d) => d,
        Err(e) => {
            log!("Discogs: {e}");
            f.error = true;
            f.notes.push("Discogs: error de red".into());
            return f;
        }
    };
    let ids: Vec<i64> = d.as_ref().and_then(|d| d["results"].as_array()).into_iter().flatten().filter_map(|r| r["id"].as_i64()).take(3).collect();
    if ids.is_empty() {
        f.notes.push("Discogs: sin ediciones en CD para ese artista y título".into());
        return f;
    }
    let rels = join_all(ids.iter().map(|id| async move { (id, get_json_h(c, &format!("https://api.discogs.com/releases/{id}"), false, Some(auth)).await) })).await;
    for (id, r) in rels {
        match r {
            Ok(Some(rel)) => {
                let mut imgs: Vec<(f64, &str)> = rel["images"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|i| i["type"] != "primary")
                    .filter_map(|i| {
                        let (w, h) = (i["width"].as_f64()?, i["height"].as_f64()?);
                        if h <= 0.0 || !(0.8..=1.25).contains(&(w / h)) {
                            return None;
                        }
                        Some(((w / h - 1.0).abs(), i["uri"].as_str()?))
                    })
                    .collect();
                imgs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                for (_, u) in imgs.into_iter().take(8) {
                    f.add("Discogs", u);
                }
            }
            Ok(None) => {}
            Err(e) => {
                log!("Discogs, edición {id}: {e}");
                f.error = true;
            }
        }
    }
    f.notes.push(format!("Discogs: {} ediciones en CD, {} imágenes para revisar", ids.len(), f.count("Discogs")));
    f
}

fn slug(artist: &str, album: &str) -> String {
    let s: String = format!("{artist}-{album}").to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    let mut out = String::new();
    for ch in s.chars() {
        if ch == '-' && out.ends_with('-') {
            continue;
        }
        out.push(ch);
    }
    out.trim_matches('-').chars().take(80).collect()
}

pub fn cache_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_cache_dir().map(|d| d.join("artwork")).map_err(|e| e.to_string())
}

/// Lee un archivo del caché por su nombre relativo (sin salirse del directorio).
pub fn read_cache(app: &AppHandle, name: &str) -> Result<Vec<u8>, String> {
    if name.contains("..") || Path::new(name).is_absolute() {
        return Err("nombre inválido".into());
    }
    fs::read(cache_root(app)?.join(name)).map_err(|e| e.to_string())
}

/// Fija una foto de CD elegida a mano (URL) para un álbum: se baja, se guarda junto al caché del álbum y se vuelve a resolver.
pub async fn set_override(app: &AppHandle, artist: &str, album: &str, url: &str) -> Result<Value, String> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("La URL tiene que empezar con http:// o https://".into());
    }
    let discogs = crate::config::get(app).get("discogsToken").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(|t| format!("Discogs token={t}"));
    let auth = if url.contains("discogs.com") { discogs.as_deref() } else { None };
    let c = client()?;
    let bytes = download_with(&c, url, auth).await.map_err(|e| format!("No se pudo bajar la imagen: {e}"))?;
    image::load_from_memory(&bytes).map_err(|e| format!("Eso no es una imagen válida: {e}"))?;
    let dir = cache_root(app)?.join(slug(artist, album));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join("override.bin"), &bytes).map_err(|e| e.to_string())?;
    fs::write(dir.join("override.url"), url).map_err(|e| e.to_string())?;
    resolve(app, artist, album, None, true, true, true).await
}

/// Quita la foto manual de un álbum (vuelve a elegir entre los proveedores en la próxima carga).
pub fn clear_override(app: &AppHandle, artist: &str, album: &str) -> Result<(), String> {
    let dir = cache_root(app)?.join(slug(artist, album));
    for f in ["override.bin", "override.url", "meta.json"] {
        let _ = fs::remove_file(dir.join(f));
    }
    Ok(())
}

pub async fn resolve(app: &AppHandle, artist: &str, album: &str, upc: Option<&str>, force: bool, want_tracks: bool, need_front: bool) -> Result<Value, String> {
    let cfg = crate::config::get(app);
    let secret = |k: &str| cfg.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    let (fanart, discogs) = (secret("fanartApiKey"), secret("discogsToken"));
    // si después del primer resultado llega un disco de mayor resolución, el widget se entera por este evento
    let app2 = app.clone();
    let updater: Updater = Arc::new(move |v| {
        let _ = app2.emit("art-update", v);
    });
    resolve_in(&cache_root(app)?, artist, album, upc, force, fanart.as_deref(), discogs.as_deref(), want_tracks, need_front, updater).await
}

// ---------------------------------------------------------------------------------------------------------------------------
// Tubería de MusicBrainz + Cover Art Archive, fuentes alternativas y la carrera entre todas
// ---------------------------------------------------------------------------------------------------------------------------

/// Resolución (diámetro del disco en la imagen original, en px) a partir de la cual no vale la pena buscar nada mejor: la salida es de 1024 px.
const HIGH_PX: u32 = 1000;
/// Un disco nuevo reemplaza al actual solo si tiene al menos este múltiplo de su resolución.
const UPGRADE_FACTOR: f32 = 1.2;
/// Cuánto sigue trabajando el resolvedor en segundo plano, buscando un disco mejor, después de entregar el primer resultado.
const BACKGROUND_MAX: Duration = Duration::from_secs(20);
/// Tope total antes de entregar lo que haya.
const FIRST_MAX: Duration = Duration::from_secs(90);
/// Cuánto se espera a la portada y la lista de canciones una vez que hay disco: si no llegaron, se entrega igual y llegan después.
const GRACE: Duration = Duration::from_millis(2500);

/// Avisa al widget cuando llega un disco mejor después del primer resultado.
pub type Updater = Arc<dyn Fn(Value) + Send + Sync>;

/// Un disco válido ya procesado, en un archivo temporal, listo para que el director lo adopte.
struct DiscCand {
    prov: &'static str,
    url: String,
    release: Option<String>,
    path: PathBuf,
    silver: f32,
    px: u32,
}

/// Lo que mandan las distintas tuberías al director de la carrera.
enum Ev {
    Group(Group),
    Front(String),
    /// `None` = no se pudo obtener
    Tracks(Option<Vec<(String, String)>>),
    Disc(DiscCand),
    Notes(Vec<String>, bool),
    /// terminó una tubería (`true` = la de MusicBrainz + Cover Art Archive)
    Done(bool),
    /// pasó el plazo de gracia
    Tick,
}

type Tx = mpsc::UnboundedSender<Ev>;

/// Contexto compartido de una resolución.
struct Ctx {
    c: reqwest::Client,
    dir: PathBuf,
    key: String,
    artist: String,
    album: String,
    upc: Option<String>,
    need_front: bool,
    want_tracks: bool,
}

/// Estado de la búsqueda de portada y disco en Cover Art Archive.
struct Scan<'a> {
    c: &'a reqwest::Client,
    dir: PathBuf,
    key: String,
    need_front: bool,
    /// mejor resolución de disco encontrada por cualquier fuente (0 = ninguno)
    quality: Arc<AtomicU32>,
    tx: Tx,
    front: Option<String>,
    cand_n: usize,
    had_error: bool,
    examined: usize,
    with_medium: usize,
    rejected: usize,
}

impl Scan<'_> {
    fn disc_enough(&self) -> bool {
        self.quality.load(Ordering::SeqCst) >= HIGH_PX
    }

    fn done(&self) -> bool {
        self.disc_enough() && (!self.need_front || self.front.is_some())
    }

    /// Valida y procesa una foto de disco; si pasa, se la ofrece al director. `true` = alcanza, dejar de probar.
    async fn try_disc(&mut self, bytes: Vec<u8>, url: &str, release: &str) -> Result<bool, String> {
        self.cand_n += 1;
        let tmp = self.dir.join(format!("disc_caa_{}.png", self.cand_n));
        let tmp2 = tmp.clone();
        match tokio::task::spawn_blocking(move || disc_crop::process_with(&bytes, &tmp2, false)).await.map_err(|e| e.to_string())? {
            Ok(st) => {
                log!("foto de CD válida de Cover Art Archive (resolución {} px, plateado {:.0} %, detalle {:.4}): {url}", st.px, st.silver * 100.0, st.detail);
                self.quality.fetch_max(st.px, Ordering::SeqCst);
                let _ = self.tx.send(Ev::Disc(DiscCand { prov: "Cover Art Archive", url: url.to_string(), release: Some(release.to_string()), path: tmp, silver: st.silver, px: st.px }));
                Ok(st.px >= HIGH_PX)
            }
            Err(why) => {
                self.rejected += 1;
                log!("foto de CD descartada ({url}): {why}");
                Ok(false)
            }
        }
    }

    /// Revisa ediciones de a tandas: los JSON de Cover Art Archive en paralelo y, por edición, portada y foto de disco en paralelo.
    async fn releases(&mut self, ids: &[String]) -> Result<(), String> {
        let c = self.c;
        for chunk in ids.chunks(6) {
            if self.done() {
                break;
            }
            let fetched = join_all(chunk.iter().map(|id| release_images(c, id))).await;
            for (id, res) in chunk.iter().zip(fetched) {
                if self.done() {
                    break;
                }
                let imgs = match res {
                    Ok(i) => i,
                    Err(e) => {
                        log!("imágenes de {id}: {e}");
                        self.had_error = true;
                        continue;
                    }
                };
                self.examined += 1;
                let mediums = imgs.get("Medium").cloned().unwrap_or_default();
                if !mediums.is_empty() {
                    self.with_medium += 1;
                }
                let front_url = if self.need_front && self.front.is_none() { imgs.get("Front").and_then(|v| v.first()).cloned() } else { None };
                let want_disc = !self.disc_enough() && !mediums.is_empty();
                let (fb, db) = tokio::join!(
                    async {
                        match &front_url {
                            Some(u) => Some(download(c, u).await),
                            None => None,
                        }
                    },
                    async {
                        if want_disc {
                            Some(download(c, &mediums[0]).await)
                        } else {
                            None
                        }
                    }
                );
                match fb {
                    Some(Ok(bytes)) => {
                        if fs::write(self.dir.join("front.jpg"), bytes).is_ok() {
                            let rel = format!("{}/front.jpg", self.key);
                            self.front = Some(rel.clone());
                            let _ = self.tx.send(Ev::Front(rel));
                        }
                    }
                    Some(Err(_)) => self.had_error = true,
                    None => {}
                }
                if want_disc {
                    // la primera foto ya está bajada; si no pasa la validación (¿es un CD?), se prueban las demás
                    let mut first = db;
                    for (i, u) in mediums.iter().enumerate() {
                        if self.disc_enough() {
                            break;
                        }
                        let got = if i == 0 { first.take().unwrap_or_else(|| Err("sin descarga".into())) } else { download(c, u).await };
                        match got {
                            Ok(bytes) => {
                                if self.try_disc(bytes, u, id).await? {
                                    break;
                                }
                            }
                            Err(_) => self.had_error = true,
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Tubería de MusicBrainz + Cover Art Archive: UPC → edición exacta → demás ediciones → búsqueda por nombre.
/// Publica los release groups apenas los conoce (los necesitan Fanart.tv y TheAudioDB), manda al director el grupo, la portada, la lista de
/// canciones y los discos que va encontrando, y sigue buscando mientras no haya un disco de resolución suficiente.
async fn main_flow(ctx: Arc<Ctx>, quality: Arc<AtomicU32>, tx: Tx, groups_tx: watch::Sender<Option<Vec<String>>>) {
    let t0 = Instant::now();
    macro_rules! lap {
        ($($t:tt)*) => { log!("{:>6} ms · {}", t0.elapsed().as_millis(), format!($($t)*)) };
    }
    let (c, artist, album) = (&ctx.c, ctx.artist.as_str(), ctx.album.as_str());
    let mut scan = Scan { c, dir: ctx.dir.clone(), key: ctx.key.clone(), need_front: ctx.need_front, quality, tx: tx.clone(), front: None, cand_n: 0, had_error: false, examined: 0, with_medium: 0, rejected: 0 };
    let mut report: Vec<String> = Vec::new();
    let mut groups: Vec<Group> = Vec::new();
    let mut sent_group = false;
    let mut tracks_started = false;
    let want_tracks = ctx.want_tracks;
    // la lista de canciones corre aparte y avisa sola cuando llega
    let start_tracks = |started: &mut bool, release: &str| {
        if want_tracks && !*started {
            *started = true;
            let (c2, id, tx2) = (c.clone(), release.to_string(), tx.clone());
            tokio::spawn(async move {
                let t = match tracklist(&c2, &id).await {
                    Ok(t) => Some(t),
                    Err(e) => {
                        log!("lista de canciones: {e}");
                        None
                    }
                };
                let _ = tx2.send(Ev::Tracks(t));
            });
        }
    };
    let send_group = |g: &Group, sent: &mut bool| {
        if !*sent {
            *sent = true;
            let _ = tx.send(Ev::Group(g.clone()));
        }
    };

    // 1) con el UPC: la edición exacta, y sus imágenes directo en Cover Art Archive
    let mut cd_task: Option<JoinHandle<Result<(Vec<CdRelease>, Option<String>), String>>> = None;
    if let Some(upc) = ctx.upc.as_deref().filter(|u| !u.is_empty()) {
        match barcode_release(c, upc, artist, album).await {
            Ok(Some(g)) => {
                lap!("UPC → edición {}", g.exact_release.as_deref().unwrap_or("?"));
                let _ = groups_tx.send(Some(vec![g.id.clone()]));
                send_group(&g, &mut sent_group);
                // las demás ediciones del grupo se piden ya (MusicBrainz), mientras se revisa la exacta en Cover Art Archive
                cd_task = Some(tokio::spawn({
                    let (c2, id) = (c.clone(), g.id.clone());
                    async move { cd_releases(&c2, &id).await }
                }));
                if let Some(exact) = g.exact_release.clone() {
                    start_tracks(&mut tracks_started, &exact);
                    report.push(format!("MusicBrainz: edición exacta del UPC {}", &exact[..8.min(exact.len())]));
                    if let Err(e) = scan.releases(&[exact]).await {
                        log!("edición exacta: {e}");
                    }
                    lap!("imágenes de la edición exacta revisadas");
                }
                groups.push(g);
            }
            Ok(None) => report.push("MusicBrainz: el UPC de Spotify no coincide con ninguna edición".into()),
            Err(e) => {
                log!("UPC: {e}");
                scan.had_error = true;
            }
        }
    }

    // 2) las demás ediciones del release group del UPC
    if !scan.done() && !groups.is_empty() {
        let g = groups[0].clone();
        let listed = match cd_task.take() {
            Some(t) => t.await.unwrap_or_else(|e| Err(e.to_string())),
            None => cd_releases(c, &g.id).await,
        };
        match listed {
            Ok((cands, first_cd)) => {
                if let Some(id) = &first_cd {
                    start_tracks(&mut tracks_started, id);
                }
                let exact = g.exact_release.clone();
                let ids: Vec<String> = cands.into_iter().filter(|r| Some(&r.id) != exact.as_ref()).take(MAX_RELEASES).map(|r| r.id).collect();
                report.push(format!("MusicBrainz {} («{}»): {} ediciones más en CD con imágenes", &g.id[..8], g.title, ids.len()));
                if let Err(e) = scan.releases(&ids).await {
                    log!("ediciones del grupo: {e}");
                }
                lap!("ediciones del grupo del UPC revisadas");
            }
            Err(e) => {
                log!("ediciones del grupo {}: {e}", g.id);
                scan.had_error = true;
            }
        }
    }
    if let Some(t) = cd_task.take() {
        t.abort(); // la edición exacta alcanzó: la lista de las demás no hace falta
    }

    // 3) búsqueda por nombre: otros release groups (o el único, si no hubo UPC)
    if !scan.done() {
        match name_groups(c, artist, album).await {
            Ok(found) => {
                lap!("búsqueda por nombre: {} grupos", found.len());
                let mut ids: Vec<String> = groups.iter().map(|g| g.id.clone()).collect();
                ids.extend(found.iter().filter(|g| !groups.iter().any(|x| x.id == g.id)).map(|g| g.id.clone()));
                let _ = groups_tx.send(Some(ids));
                if let Some(g) = found.first() {
                    send_group(g, &mut sent_group);
                }
                for g in found {
                    if scan.done() {
                        break;
                    }
                    if groups.iter().any(|x| x.id == g.id) {
                        continue;
                    }
                    match cd_releases(c, &g.id).await {
                        Ok((cands, first_cd)) => {
                            if let Some(id) = first_cd.as_ref().or(cands.first().map(|r| &r.id)) {
                                start_tracks(&mut tracks_started, id);
                            }
                            let ids: Vec<String> = cands.into_iter().take(MAX_RELEASES).map(|r| r.id).collect();
                            report.push(format!("MusicBrainz {} («{}»): {} ediciones en CD con imágenes", &g.id[..8], g.title, ids.len()));
                            if let Err(e) = scan.releases(&ids).await {
                                log!("ediciones del grupo: {e}");
                            }
                        }
                        Err(e) => {
                            log!("ediciones del grupo {}: {e}", g.id);
                            scan.had_error = true;
                            report.push(format!("MusicBrainz {}: error de red", &g.id[..8]));
                        }
                    }
                    groups.push(g);
                }
                lap!("grupos por nombre revisados");
            }
            Err(e) => {
                log!("búsqueda por nombre: {e}");
                scan.had_error = true;
            }
        }
    }
    // si no se supo ningún grupo, los que esperan el release group (Fanart.tv, TheAudioDB) dejan de esperar
    let _ = groups_tx.send(Some(groups.iter().map(|g| g.id.clone()).collect()));
    if want_tracks && !tracks_started {
        let _ = tx.send(Ev::Tracks(None)); // nunca se supo qué edición pedir
    }
    report.push(format!(
        "Cover Art Archive: {} ediciones revisadas, {} con foto de disco{}",
        scan.examined,
        scan.with_medium,
        if scan.rejected > 0 { format!(" ({} descartadas por no parecer un CD)", scan.rejected) } else { String::new() }
    ));
    let _ = tx.send(Ev::Notes(report, scan.had_error));
    let _ = tx.send(Ev::Done(true));
}

// ---------------------------------------------------------------------------------------------------------------------------
// Fuentes alternativas del disco, en carrera con Cover Art Archive
// ---------------------------------------------------------------------------------------------------------------------------

/// Baja todas las imágenes candidatas a la vez y las procesa a la vez. Devuelve las válidas (en el orden de entrada), cuántas se descartaron
/// y si algo falló por red. Si ya hay un disco de resolución suficiente, no baja nada.
async fn fetch_and_process(c: &reqwest::Client, cands: &[(&'static str, String)], auth: Option<&str>, strict: bool, dir: &Path, tag: &str, quality: &AtomicU32) -> (Vec<(usize, disc_crop::Stats, PathBuf)>, usize, bool) {
    if cands.is_empty() || quality.load(Ordering::SeqCst) >= HIGH_PX {
        return (Vec::new(), 0, false);
    }
    let downloads = join_all(cands.iter().map(|(_, u)| download_with(c, u, auth))).await;
    let mut error = false;
    let mut jobs = Vec::new();
    for (n, ((prov, url), dl)) in cands.iter().zip(downloads).enumerate() {
        match dl {
            Ok(bytes) => {
                let out = dir.join(format!("alt_{tag}_{n}.png"));
                let out2 = out.clone();
                jobs.push((n, out, tokio::task::spawn_blocking(move || disc_crop::process_with(&bytes, &out2, strict))));
            }
            Err(_) => {
                error = true;
                log!("sin imagen de {prov}: {url}");
            }
        }
    }
    let (mut valid, mut rejected) = (Vec::new(), 0usize);
    for (n, out, job) in jobs {
        match job.await {
            Ok(Ok(st)) => {
                log!("disco de {} válido (resolución {} px, plateado {:.0} %, detalle {:.4}): {}", cands[n].0, st.px, st.silver * 100.0, st.detail, cands[n].1);
                valid.push((n, st, out));
            }
            Ok(Err(why)) => {
                rejected += 1;
                log!("disco de {} descartado ({}): {why}", cands[n].0, cands[n].1);
            }
            Err(_) => {}
        }
    }
    (valid, rejected, error)
}

/// Ofrece al director el mejor de los discos válidos de una fuente y limpia los demás archivos.
fn offer(tx: &Tx, quality: &AtomicU32, cands: &[(&'static str, String)], valid: Vec<(usize, disc_crop::Stats, PathBuf)>, pick_printed: bool) {
    let best = if pick_printed {
        valid.iter().enumerate().min_by(|a, b| (a.1).1.silver.partial_cmp(&(b.1).1.silver).unwrap_or(std::cmp::Ordering::Equal)).map(|(i, _)| i)
    } else {
        valid.iter().enumerate().next().map(|(i, _)| i)
    };
    for (i, (n, st, path)) in valid.into_iter().enumerate() {
        if Some(i) == best {
            quality.fetch_max(st.px, Ordering::SeqCst);
            let _ = tx.send(Ev::Disc(DiscCand { prov: cands[n].0, url: cands[n].1.clone(), release: None, path, silver: st.silver, px: st.px }));
        } else {
            let _ = fs::remove_file(path);
        }
    }
}

/// TheAudioDB por nombre del álbum.
async fn alt_audiodb(c: reqwest::Client, artist: String, album: String, dir: PathBuf, quality: Arc<AtomicU32>, tx: Tx) {
    let f = audiodb_by_name(&c, &artist, &album).await;
    let (valid, rejected, err) = fetch_and_process(&c, &f.urls, None, false, &dir, "audiodb", &quality).await;
    let note = format!("TheAudioDB (por nombre): {} discos{}", f.urls.len(), if rejected > 0 { format!(", {rejected} descartados") } else { String::new() });
    offer(&tx, &quality, &f.urls, valid, false);
    let _ = tx.send(Ev::Notes(vec![note], f.error || err));
    let _ = tx.send(Ev::Done(false));
}

/// Discogs: se evalúan todas las fotos candidatas y se ofrece la menos plateada (la del lado impreso).
async fn alt_discogs(c: reqwest::Client, artist: String, album: String, auth: String, dir: PathBuf, quality: Arc<AtomicU32>, tx: Tx) {
    let f = discogs(&c, &artist, &album, &auth).await;
    let (valid, rejected, err) = fetch_and_process(&c, &f.urls, Some(&auth), true, &dir, "discogs", &quality).await;
    let mut notes = f.notes.clone();
    if rejected > 0 {
        notes.push(format!("Discogs: {rejected} imágenes descartadas por no parecer un CD (o ser el lado de datos)"));
    }
    offer(&tx, &quality, &f.urls, valid, true);
    let _ = tx.send(Ev::Notes(notes, f.error || err));
    let _ = tx.send(Ev::Done(false));
}

/// Fanart.tv y TheAudioDB por release group: esperan a que MusicBrainz identifique el grupo.
async fn alt_by_mbid(c: reqwest::Client, mut groups_rx: watch::Receiver<Option<Vec<String>>>, fanart_key: Option<String>, dir: PathBuf, quality: Arc<AtomicU32>, tx: Tx) {
    let ids = loop {
        if let Some(ids) = groups_rx.borrow_and_update().clone() {
            break ids;
        }
        if groups_rx.changed().await.is_err() {
            let _ = tx.send(Ev::Done(false)); // la tubería de MusicBrainz terminó sin identificar nada
            return;
        }
    };
    if ids.is_empty() {
        let _ = tx.send(Ev::Done(false));
        return;
    }
    let (fan, adb) = tokio::join!(
        async {
            match &fanart_key {
                Some(k) => Some(fanart(&c, &ids, k).await),
                None => None,
            }
        },
        audiodb_by_mbid(&c, &ids)
    );
    let mut notes = Vec::new();
    let mut all = Found::default();
    match fan {
        Some(f) => {
            notes.push(format!("Fanart.tv: {} discos", f.count("Fanart.tv")));
            all.merge(f);
        }
        None => notes.push("Fanart.tv: sin clave (es opcional: se carga en Ajustes)".into()),
    }
    notes.push(format!("TheAudioDB (por release group): {} discos", adb.count("TheAudioDB")));
    all.merge(adb);
    let (valid, rejected, err) = fetch_and_process(&c, &all.urls, None, false, &dir, "mbid", &quality).await;
    if rejected > 0 {
        notes.push(format!("Fanart.tv / TheAudioDB (release group): {rejected} imágenes descartadas por no parecer un CD"));
    }
    offer(&tx, &quality, &all.urls, valid, false);
    let _ = tx.send(Ev::Notes(notes, all.error || err));
    let _ = tx.send(Ev::Done(false));
}

// ---------------------------------------------------------------------------------------------------------------------------
// El director de la carrera
// ---------------------------------------------------------------------------------------------------------------------------

/// Una resolución a la vez por álbum (si no, dos carreras del mismo álbum se pisarían los archivos).
static LOCKS: std::sync::Mutex<Option<HashMap<String, Arc<Mutex<()>>>>> = std::sync::Mutex::new(None);

fn lock_for(key: &str) -> Arc<Mutex<()>> {
    let mut g = LOCKS.lock().unwrap();
    g.get_or_insert_with(HashMap::new).entry(key.to_string()).or_default().clone()
}

/// Una recarga forzada de un álbum cancela el trabajo en segundo plano que siga en curso para ese álbum, en vez de esperarlo.
static CANCELS: std::sync::Mutex<Option<HashMap<String, Arc<Notify>>>> = std::sync::Mutex::new(None);

fn cancel_for(key: &str) -> Arc<Notify> {
    let mut g = CANCELS.lock().unwrap();
    g.get_or_insert_with(HashMap::new).entry(key.to_string()).or_default().clone()
}

/// Espera a que termine el trabajo en segundo plano de un álbum (para las pruebas).
#[allow(dead_code)]
pub async fn wait_idle(artist: &str, album: &str) {
    let lock = lock_for(&slug(artist, album));
    let _g = lock.lock().await;
}

struct Adopted {
    prov: &'static str,
    url: String,
    release: Option<String>,
    px: u32,
}

fn cleanup_temp(dir: &Path) {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("alt_") || n.starts_with("disc_caa") || n.starts_with("disc_try_") {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

/// Núcleo del resolvedor, sobre un directorio de caché cualquiera (así se prueba sin la app).
/// `want_tracks`: pedir a MusicBrainz la lista de canciones (no hace falta si Spotify ya la trajo).
/// `need_front`: buscar la portada (no hace falta si Spotify ya trajo la suya).
///
/// El disco sale de una carrera: MusicBrainz + Cover Art Archive, Discogs, TheAudioDB y Fanart.tv arrancan juntos. El primer resultado se
/// devuelve apenas hay disco (y lo demás que se pidió); las otras fuentes NO se cortan: siguen en segundo plano, y si alguna entrega un
/// disco de mayor resolución reemplaza al actual y se avisa por `on_update`.
#[allow(clippy::too_many_arguments)]
pub async fn resolve_in(
    root: &Path,
    artist: &str,
    album: &str,
    upc: Option<&str>,
    force: bool,
    fanart_key: Option<&str>,
    discogs_token: Option<&str>,
    want_tracks: bool,
    need_front: bool,
    on_update: Updater,
) -> Result<Value, String> {
    let t0 = Instant::now();
    macro_rules! lap {
        ($($t:tt)*) => { log!("{:>6} ms · {}", t0.elapsed().as_millis(), format!($($t)*)) };
    }
    let key = slug(artist, album);
    let dir = root.join(&key);
    let meta = dir.join("meta.json");
    let cancel = cancel_for(&key);
    if force {
        cancel.notify_waiters(); // si hay trabajo en segundo plano de este álbum, que termine ya
    }
    let guard = lock_for(&key).lock_owned().await;
    if !force {
        if let Ok(s) = fs::read_to_string(&meta) {
            if let Ok(v) = serde_json::from_str::<Value>(&s) {
                // el caché vale si tiene todo lo que se pide ahora
                let fresh = v["disc"].is_string() || now_secs().saturating_sub(v["ts"].as_u64().unwrap_or(0)) < NEGATIVE_TTL_SECS;
                let ok = fresh && v["v"].as_i64() == Some(CACHE_VERSION) && (!want_tracks || v["tracks_ok"].as_bool() == Some(true)) && (!need_front || v["front_checked"].as_bool() == Some(true));
                if ok {
                    lap!("{artist} — {album}: desde el caché");
                    return Ok(v);
                }
            }
        }
    }
    lap!("{artist} — {album}: buscando (upc {:?}, canciones {want_tracks}, portada {need_front})", upc);
    let c = client()?;
    let discogs_auth: Option<String> = discogs_token.map(|t| format!("Discogs token={t}"));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    cleanup_temp(&dir); // restos de una resolución anterior interrumpida

    // foto manual elegida por el usuario para este álbum (Ajustes): tiene prioridad sobre todas las fuentes
    let mut adopted: Option<Adopted> = None;
    let mut report: Vec<String> = Vec::new();
    if let Ok(bytes) = fs::read(dir.join("override.bin")) {
        let out = dir.join("disc.png");
        match tokio::task::spawn_blocking(move || disc_crop::process_manual(&bytes, &out)).await.map_err(|e| e.to_string())? {
            Ok(_) => {
                adopted = Some(Adopted { prov: "Foto manual", url: fs::read_to_string(dir.join("override.url")).unwrap_or_default(), release: None, px: u32::MAX });
                report.push("Foto del CD manual: se usa la que elegiste".into());
                lap!("foto manual del CD aplicada");
            }
            Err(e) => report.push(format!("Foto del CD manual: {e}")),
        }
    }
    let manual = adopted.is_some();

    // las tuberías
    let quality = Arc::new(AtomicU32::new(if manual { u32::MAX } else { 0 }));
    let (tx, mut rx) = mpsc::unbounded_channel::<Ev>();
    let (groups_tx, groups_rx) = watch::channel::<Option<Vec<String>>>(None);
    let ctx = Arc::new(Ctx { c: c.clone(), dir: dir.clone(), key: key.clone(), artist: artist.to_string(), album: album.to_string(), upc: upc.map(String::from), need_front, want_tracks });
    let mut producers: JoinSet<()> = JoinSet::new();
    let mut remaining = 1usize;
    producers.spawn(main_flow(ctx.clone(), quality.clone(), tx.clone(), groups_tx));
    if !manual {
        producers.spawn(alt_audiodb(c.clone(), artist.to_string(), album.to_string(), dir.clone(), quality.clone(), tx.clone()));
        remaining += 1;
        match &discogs_auth {
            Some(auth) => {
                producers.spawn(alt_discogs(c.clone(), artist.to_string(), album.to_string(), auth.clone(), dir.clone(), quality.clone(), tx.clone()));
                remaining += 1;
            }
            None => report.push("Discogs: sin token (es opcional: se carga en Ajustes)".into()),
        }
        producers.spawn(alt_by_mbid(c.clone(), groups_rx, fanart_key.map(String::from), dir.clone(), quality.clone(), tx.clone()));
        remaining += 1;
    }
    drop(tx);

    // el director: junta lo que llega, entrega el primer resultado cuando está lo pedido y sigue buscando un disco mejor
    let (first_tx, first_rx) = oneshot::channel::<Value>();
    let meta2 = meta.clone();
    let (artist2, album2, key2, dir2) = (artist.to_string(), album.to_string(), key.clone(), dir.clone());
    tokio::spawn(async move {
        let _guard = guard; // el álbum queda bloqueado hasta que termine el trabajo en segundo plano
        let _producers = producers; // al terminar el director se cancela lo que quede
        let (mut group, mut front, mut tracks): (Option<Group>, Option<String>, Option<Option<Vec<(String, String)>>>) = (None, None, if want_tracks { None } else { Some(Some(Vec::new())) });
        let (mut had_error, mut main_done) = (false, false);
        let mut first_tx = Some(first_tx);
        // si MusicBrainz no identifica el álbum pero hay disco (foto manual o de otro proveedor), se entrega igual con el nombre que se pidió
        let fallback = Group { id: String::new(), title: album2.clone(), artist: artist2.clone(), year: None, exact_release: None };
        let started = Instant::now();
        let mut first_at: Option<Instant> = None;
        let mut disc_first_at: Option<Instant> = None;
        let mut grace_fired = false;
        let state_json = |group: &Group, front: &Option<String>, tracks: &Option<Option<Vec<(String, String)>>>, adopted: &Option<Adopted>, report: &[String], had_error: bool| -> Value {
            let tr = tracks.clone().flatten().unwrap_or_default();
            let front_rel = if front.is_some() { front.clone() } else if !need_front && dir2.join("front.jpg").exists() { Some(format!("{key2}/front.jpg")) } else { None };
            json!({
                "v": CACHE_VERSION, "found": true, "artist": group.artist, "title": group.title, "year": group.year, "release_group": group.id,
                "tracks": tr, "tracks_ok": matches!(tracks, Some(Some(_))) || !want_tracks, "front_checked": need_front,
                "disc": adopted.as_ref().map(|_| format!("{key2}/disc.png")), "disc_source": adopted.as_ref().map(|a| a.prov),
                "disc_url": adopted.as_ref().map(|a| a.url.clone()), "disc_release": adopted.as_ref().and_then(|a| a.release.clone()),
                "disc_px": adopted.as_ref().map(|a| a.px.min(100_000)), "front": front_rel, "report": report, "incomplete": had_error,
                "qartist": artist2, "qalbum": album2, "ts": now_secs(),
            })
        };
        loop {
            let deadline = match first_at {
                Some(t) => t + BACKGROUND_MAX,
                None => started + FIRST_MAX,
            };
            let grace_deadline = disc_first_at.map(|t| t + GRACE);
            let ev = tokio::select! {
                ev = rx.recv() => ev,
                _ = tokio::time::sleep_until(deadline.into()) => None,
                _ = cancel.notified() => None,
                _ = tokio::time::sleep_until(grace_deadline.unwrap_or(deadline).into()), if first_tx.is_some() && grace_deadline.is_some() && !grace_fired => {
                    grace_fired = true;
                    Some(Ev::Tick)
                }
            };
            let Some(ev) = ev else { break };
            let mut late = false; // llegó portada o canciones después del primer resultado
            match ev {
                Ev::Tick => {}
                Ev::Group(g) => {
                    if group.is_none() {
                        group = Some(g);
                    }
                }
                Ev::Front(p) => {
                    front = Some(p);
                    late = true;
                }
                Ev::Tracks(t) => {
                    tracks = Some(t);
                    late = true;
                }
                Ev::Notes(n, err) => {
                    report.extend(n);
                    had_error |= err;
                }
                Ev::Done(is_main) => {
                    remaining -= 1;
                    if is_main {
                        main_done = true;
                    }
                }
                Ev::Disc(cand) => {
                    let better = match &adopted {
                        None => true,
                        Some(a) => (cand.px as f32) > (a.px as f32) * UPGRADE_FACTOR,
                    };
                    if better && fs::rename(&cand.path, dir2.join("disc.png")).is_ok() {
                        lap!("disco adoptado de {} (resolución {} px){}", cand.prov, cand.px, if adopted.is_some() { " — mejora" } else { "" });
                        let _ = cand.silver;
                        adopted = Some(Adopted { prov: cand.prov, url: cand.url, release: cand.release, px: cand.px });
                        if disc_first_at.is_none() {
                            disc_first_at = Some(Instant::now());
                        }
                        if first_at.is_some() {
                            // llegó después del primer resultado: se avisa para que el widget cambie la imagen
                            if let Some(g) = &group {
                                on_update(state_json(g, &front, &tracks, &adopted, &report, had_error));
                            }
                        }
                    } else {
                        let _ = fs::remove_file(&cand.path);
                    }
                }
            }
            // portada o lista de canciones que llegan después del primer resultado: se avisa para que el widget las aplique
            if late && first_at.is_some() {
                if let Some(g) = &group {
                    on_update(state_json(g, &front, &tracks, &adopted, &report, had_error));
                }
            }
            // ¿se puede entregar el primer resultado? Con disco, apenas está lo demás o pasó el plazo de gracia; sin disco, cuando terminaron todas
            if first_tx.is_some() {
                let complete = (!need_front || front.is_some() || main_done) && (tracks.is_some() || main_done);
                let grace_over = disc_first_at.is_some_and(|t| t.elapsed() >= GRACE);
                let ready_found = group.is_some() && ((adopted.is_some() && (complete || grace_over)) || (remaining == 0 && complete));
                if ready_found {
                    let g = group.clone().unwrap();
                    first_at = Some(Instant::now());
                    let _ = first_tx.take().unwrap().send(state_json(&g, &front, &tracks, &adopted, &report, had_error));
                } else if main_done && group.is_none() && (adopted.is_some() || remaining == 0) {
                    first_at = Some(Instant::now());
                    if adopted.is_some() {
                        group = Some(fallback.clone());
                        let _ = first_tx.take().unwrap().send(state_json(&fallback, &front, &tracks, &adopted, &report, had_error));
                    } else {
                        let _ = first_tx.take().unwrap().send(json!({ "found": false, "artist": artist2, "title": album2, "tracks": [], "incomplete": had_error }));
                        break;
                    }
                }
            }
            if remaining == 0 {
                break;
            }
        }
        // se entrega lo que haya si se acabó el tiempo antes del primer resultado
        if let Some(tx1) = first_tx.take() {
            let v = match &group {
                Some(g) => state_json(g, &front, &tracks, &adopted, &report, had_error),
                None => json!({ "found": false, "artist": artist2, "title": album2, "tracks": [], "incomplete": true }),
            };
            let _ = tx1.send(v);
        }
        // cierre: sin temporales, sin archivos viejos, y el resultado final al caché
        cleanup_temp(&dir2);
        if adopted.is_none() {
            let _ = fs::remove_file(dir2.join("disc.png"));
        }
        if front.is_none() && need_front {
            let _ = fs::remove_file(dir2.join("front.jpg"));
        }
        if let Some(g) = &group {
            let fin = state_json(g, &front, &tracks, &adopted, &report, had_error);
            lap!("{artist2} — {album2}: final · disco={} ({} px) portada={} error_red={had_error}", adopted.as_ref().map(|a| a.prov).unwrap_or("ninguno"), adopted.as_ref().map(|a| a.px.min(100_000)).unwrap_or(0), front.is_some() || fin["front"].is_string());
            if !had_error {
                let _ = fs::write(&meta2, serde_json::to_string_pretty(&fin).unwrap_or_default());
            }
        }
    });
    let first = first_rx.await.map_err(|_| "la resolución se interrumpió".to_string())?;
    lap!("{artist} — {album}: primer resultado (disco={} portada={} error_red={})", first["disc"].is_string(), first["front"].is_string(), first["incomplete"]);
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> (Option<String>, Option<String>) {
        (std::env::var("FANART_KEY").ok(), std::env::var("DISCOGS_TOKEN").ok())
    }

    /// Updater que junta los avisos de mejora.
    fn collector() -> (Updater, Arc<std::sync::Mutex<Vec<Value>>>) {
        let got = Arc::new(std::sync::Mutex::new(Vec::new()));
        let g2 = got.clone();
        (Arc::new(move |v| g2.lock().unwrap().push(v)), got)
    }

    fn read_meta(dir: &Path, artist: &str, album: &str) -> Value {
        fs::read_to_string(dir.join(slug(artist, album)).join("meta.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
    }

    /// Prueba de punta a punta contra la red real. `cargo test resolver_real -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn resolver_real() {
        let dir = std::env::temp_dir().join("jewelcase-test-arte");
        let _ = fs::remove_dir_all(&dir);
        for (artist, album) in [("Bersuit Vergarabat", "Libertinaje"), ("Charly García", "Clics modernos")] {
            let r = resolve_in(&dir, artist, album, None, true, None, None, true, true, collector().0).await.expect("resolve");
            println!("{r}");
            assert!(r["disc"].is_string(), "{artist} — {album}: no encontró la foto del CD");
            wait_idle(artist, album).await;
        }
    }

    /// Tiempo hasta el primer resultado y hasta el final (con las mejoras), sin caché y con UPC como llega de Spotify:
    /// `cargo test tiempos -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn tiempos() {
        let (fanart, discogs) = keys();
        let (mut total_first, mut total_all) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        for (artist, album, upc) in [
            ("Charly García", "Clics modernos", Some("00602498002155")),
            ("Soda Stereo", "Canción animal", None),
            ("Bersuit Vergarabat", "Libertinaje", None),
            ("Tan Biónica", "Obsesionario", Some("8429006350216")),
        ] {
            let dir = std::env::temp_dir().join("jewelcase-test-tiempos");
            let _ = fs::remove_dir_all(&dir);
            let (up, got) = collector();
            let t = Instant::now();
            let r = resolve_in(&dir, artist, album, upc, true, fanart.as_deref(), discogs.as_deref(), upc.is_none(), upc.is_none(), up).await.expect("resolve");
            let first = t.elapsed();
            wait_idle(artist, album).await;
            let all = t.elapsed();
            total_first += first;
            total_all += all;
            let fin = read_meta(&dir, artist, album);
            println!(
                "TIEMPO {artist} — {album}: primer resultado {:.1} s (disco={} fuente={} {} px) | final {:.1} s (fuente={} {} px) | mejoras={}",
                first.as_secs_f32(), r["disc"].is_string(), r["disc_source"], r["disc_px"], all.as_secs_f32(), fin["disc_source"], fin["disc_px"], got.lock().unwrap().len()
            );
        }
        println!("TIEMPO total: primer resultado {:.1} s, final {:.1} s", total_first.as_secs_f32(), total_all.as_secs_f32());
    }

    /// Diagnóstico de un álbum: informe del resolvedor y, si hay IMG_URL (de Discogs), por qué pasa o no la validación estricta.
    /// `ARTIST=... ALBUM=... IMG_URL=... cargo test diagnostico -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn diagnostico() {
        let (fanart, discogs_t) = keys();
        let (artist, album) = (std::env::var("ARTIST").unwrap(), std::env::var("ALBUM").unwrap());
        let dir = std::env::temp_dir().join("jewelcase-test-diag");
        let _ = fs::remove_dir_all(&dir);
        let r = resolve_in(&dir, &artist, &album, std::env::var("UPC").ok().as_deref(), true, fanart.as_deref(), discogs_t.as_deref(), true, true, collector().0).await.expect("resolve");
        wait_idle(&artist, &album).await;
        println!("DIAG disco={} fuente={} px={} informe={}", r["disc"], r["disc_source"], r["disc_px"], read_meta(&dir, &artist, &album)["report"]);
        if let Ok(url) = std::env::var("IMG_URL") {
            let auth = discogs_t.as_ref().map(|t| format!("Discogs token={t}"));
            let bytes = download_with(&client().unwrap(), &url, auth.as_deref()).await.expect("descarga");
            let img = image::load_from_memory(&bytes).unwrap();
            println!("DIAG imagen {}x{}", img.width(), img.height());
            let out = dir.join("diag_strict.png");
            println!("DIAG estricto: {:?}", disc_crop::process_with(&bytes, &out, true).map(|s| (s.px, s.silver, s.detail)));
            let out2 = dir.join("diag_manual.png");
            println!("DIAG manual: {:?}", disc_crop::process_manual(&bytes, &out2).map(|s| (s.px, s.silver, s.detail)));
        }
    }

    /// Baja las imágenes candidatas de Discogs de un álbum a una carpeta (OUT_DIR) para calibrar la detección:
    /// `ARTIST=... ALBUM=... SAVE_DIR=... DISCOGS_TOKEN=... cargo test bajar_discogs -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn bajar_discogs() {
        let c = client().unwrap();
        let auth = format!("Discogs token={}", std::env::var("DISCOGS_TOKEN").unwrap());
        let out = PathBuf::from(std::env::var("SAVE_DIR").unwrap());
        fs::create_dir_all(&out).unwrap();
        let tag = std::env::var("TAG").unwrap_or_else(|_| "d".into());
        let f = discogs(&c, &std::env::var("ARTIST").unwrap(), &std::env::var("ALBUM").unwrap(), &auth).await;
        for (n, (_, u)) in f.urls.iter().enumerate() {
            match download_with(&c, u, Some(&auth)).await {
                Ok(b) => {
                    let p = out.join(format!("{tag}_{n}.jpg"));
                    println!("ESCRITO {} ({} bytes)", p.display(), b.len());
                    fs::write(p, b).unwrap();
                }
                Err(e) => println!("FALLO {n}: {e}"),
            }
        }
        println!("BAJADAS {}", f.urls.len());
    }

    /// Con una foto manual, el disco se entrega aunque MusicBrainz no conozca el álbum: `MANUAL_URL=... cargo test manual_sin_musicbrainz`.
    #[tokio::test]
    #[ignore]
    async fn manual_sin_musicbrainz() {
        let dir = std::env::temp_dir().join("jewelcase-test-manual2");
        let _ = fs::remove_dir_all(&dir);
        let (artist, album) = ("Artista que no existe zzqx", "Álbum que no existe zzqx");
        let url = std::env::var("MANUAL_URL").expect("MANUAL_URL");
        let auth = std::env::var("DISCOGS_TOKEN").ok().map(|t| format!("Discogs token={t}"));
        let bytes = download_with(&client().unwrap(), &url, auth.as_deref()).await.expect("descarga");
        let adir = dir.join(slug(artist, album));
        fs::create_dir_all(&adir).unwrap();
        fs::write(adir.join("override.bin"), &bytes).unwrap();
        fs::write(adir.join("override.url"), &url).unwrap();
        let r = resolve_in(&dir, artist, album, None, true, None, None, false, false, collector().0).await.expect("resolve");
        println!("SINMB found={} disco={} fuente={}", r["found"], r["disc"], r["disc_source"]);
        assert_eq!(r["found"], true);
        assert_eq!(r["disc_source"], "Foto manual");
        assert!(dir.join(slug(artist, album)).join("disc.png").exists());
    }

    /// Foto manual de punta a punta: baja la URL (MANUAL_URL, con DISCOGS_TOKEN si es de Discogs), la fija para el álbum y resuelve.
    #[tokio::test]
    #[ignore]
    async fn manual_obsesionario() {
        let dir = std::env::temp_dir().join("jewelcase-test-arte4");
        let _ = fs::remove_dir_all(&dir);
        let url = std::env::var("MANUAL_URL").expect("MANUAL_URL");
        let auth = std::env::var("DISCOGS_TOKEN").ok().map(|t| format!("Discogs token={t}"));
        let bytes = download_with(&client().unwrap(), &url, auth.as_deref()).await.expect("descarga");
        let adir = dir.join(slug("Tan Biónica", "Obsesionario"));
        fs::create_dir_all(&adir).unwrap();
        fs::write(adir.join("override.bin"), &bytes).unwrap();
        fs::write(adir.join("override.url"), &url).unwrap();
        let r = resolve_in(&dir, "Tan Biónica", "Obsesionario", None, true, None, None, true, true, collector().0).await.expect("resolve");
        println!("MANUAL disco={} fuente={} informe={}", r["disc"], r["disc_source"], r["report"]);
        assert_eq!(r["disc_source"], "Foto manual");
    }
}
