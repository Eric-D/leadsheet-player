//! Cloud "space": a shareable library backed by the user's own Supabase. The
//! QR code / URL carries the WHOLE config (Supabase URL + anon key + space key),
//! so the app is generic — scanning the code "logs in" to that space. The local
//! IndexedDB library is the working copy / offline cache.

use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = cloud, catch)]
    async fn push(url: &str, anon: &str, space: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_namespace = cloud, catch)]
    async fn pull(url: &str, anon: &str, space: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_namespace = cloudCfg)]
    fn load() -> String;
    #[wasm_bindgen(js_namespace = cloudCfg)]
    fn save(s: &str);
}

#[derive(Clone, Default)]
pub struct Config {
    pub url: String,
    pub anon: String,
    pub space: String,
}

impl Config {
    pub fn is_set(&self) -> bool {
        !self.url.is_empty() && !self.anon.is_empty() && !self.space.is_empty()
    }
}

fn parse_json(s: &str) -> Config {
    let mut c = Config::default();
    if let Ok(v) = js_sys::JSON::parse(s) {
        let g = |k: &str| {
            js_sys::Reflect::get(&v, &JsValue::from_str(k))
                .ok()
                .and_then(|x| x.as_string())
                .unwrap_or_default()
        };
        c.url = g("url");
        c.anon = g("anon");
        c.space = g("space");
    }
    c
}

fn to_json(c: &Config) -> String {
    let o = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&o, &"url".into(), &c.url.as_str().into());
    let _ = js_sys::Reflect::set(&o, &"anon".into(), &c.anon.as_str().into());
    let _ = js_sys::Reflect::set(&o, &"space".into(), &c.space.as_str().into());
    js_sys::JSON::stringify(&o)
        .ok()
        .and_then(|x| x.as_string())
        .unwrap_or_default()
}

pub fn load_config() -> Config {
    parse_json(&load())
}
pub fn save_config(c: &Config) {
    save(&to_json(c));
}

/// Encode the config into a URL-safe base64 string for the `#c=` fragment / QR.
pub fn to_fragment(c: &Config) -> String {
    let json = to_json(c);
    let b64 = web_sys::window()
        .and_then(|w| w.btoa(&json).ok())
        .unwrap_or_default();
    b64.replace('+', "-").replace('/', "_")
}

pub fn from_fragment(frag: &str) -> Option<Config> {
    let b64 = frag.replace('-', "+").replace('_', "/");
    let json = web_sys::window()?.atob(&b64).ok()?;
    let c = parse_json(&json);
    c.is_set().then_some(c)
}

/// Full share URL (current page, minus any existing fragment, plus `#c=...`).
pub fn share_url(c: &Config) -> String {
    let loc = web_sys::window().map(|w| w.location());
    let base = loc
        .and_then(|l| l.href().ok())
        .unwrap_or_default();
    let base = base.split('#').next().unwrap_or("").to_string();
    format!("{base}#c={}", to_fragment(c))
}

/// If the page was opened with a `#c=<config>` fragment, return it.
pub fn config_in_url() -> Option<Config> {
    let hash = web_sys::window()?.location().hash().ok()?;
    let frag = hash.trim_start_matches('#');
    let val = frag.strip_prefix("c=")?;
    from_fragment(val)
}

pub type StatusInbox = Rc<RefCell<Option<String>>>;

/// Upload the local library to the space.
pub fn sync_push(c: Config, status: StatusInbox, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        let msg = match push(&c.url, &c.anon, &c.space).await {
            Ok(n) => format!("Envoyé ({} morceaux)", n.as_f64().unwrap_or(0.0) as i64),
            Err(_) => "Échec de l'envoi (vérifie l'URL/clé/espace).".into(),
        };
        *status.borrow_mut() = Some(msg);
        ctx.request_repaint();
    });
}

/// Download the space into the local library, then refresh the list.
pub fn sync_pull(c: Config, status: StatusInbox, lib_inbox: crate::library::LibInbox, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        let msg = match pull(&c.url, &c.anon, &c.space).await {
            Ok(n) => format!("Récupéré ({} nouveaux morceaux)", n.as_f64().unwrap_or(0.0) as i64),
            Err(_) => "Échec de la récupération (vérifie l'URL/clé/espace).".into(),
        };
        *status.borrow_mut() = Some(msg);
        crate::library::refresh(lib_inbox, ctx.clone());
        ctx.request_repaint();
    });
}

/// A short random space key.
pub fn random_key() -> String {
    let mut buf = [0u8; 12];
    let _ = web_sys::window()
        .and_then(|w| w.crypto().ok())
        .map(|c| c.get_random_values_with_u8_array(&mut buf));
    const HEX: &[u8; 16] = b"0123456789abcdef";
    buf.iter().flat_map(|b| [HEX[(b >> 4) as usize], HEX[(b & 15) as usize]]).map(|c| c as char).collect()
}

/// Draw a QR code of `data` into the UI (pure Rust, no JS).
pub fn draw_qr(ui: &mut egui::Ui, data: &str, size: f32) {
    let Ok(code) = qrcode::QrCode::new(data.as_bytes()) else {
        ui.label("QR indisponible (donnée trop longue).");
        return;
    };
    let modules = code.width();
    let colors = code.to_colors();
    let quiet = 3usize;
    let total = modules + quiet * 2;
    let cell = (size / total as f32).max(1.0);
    let side = cell * total as f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, egui::Color32::WHITE);
    for y in 0..modules {
        for x in 0..modules {
            if colors[y * modules + x] == qrcode::Color::Dark {
                let min = rect.min + egui::vec2((x + quiet) as f32 * cell, (y + quiet) as f32 * cell);
                p.rect_filled(egui::Rect::from_min_size(min, egui::vec2(cell, cell)), 0.0, egui::Color32::BLACK);
            }
        }
    }
}
