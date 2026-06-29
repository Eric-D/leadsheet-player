//! Local song library backed by the browser's IndexedDB (via the `window.localLib`
//! glue in index.html). No server, no account — songs persist across sessions on
//! this device. Metadata (extracted by our own parser) is stored alongside the
//! raw bytes so the list renders without re-parsing.

use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

#[derive(Clone)]
pub struct LibEntry {
    pub id: f64,
    pub name: String,
    pub title: String,
    pub key: String,
    pub tempo: u16,
    pub style: String,
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(catch, js_namespace = localLib)]
    async fn list() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_namespace = localLib)]
    async fn get(id: f64) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_namespace = localLib)]
    async fn save(
        name: &str,
        bytes: &[u8],
        title: &str,
        key: &str,
        tempo: f64,
        style: &str,
    ) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_namespace = localLib)]
    async fn remove(id: f64) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_namespace = localLib)]
    async fn rename(id: f64, title: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = downloadFile)]
    fn download_file(name: &str, bytes: &[u8]);
}

/// True if the IndexedDB-backed library glue is present (always, in our build).
pub fn available() -> bool {
    js_sys::Reflect::has(&js_sys::global(), &JsValue::from_str("localLib")).unwrap_or(false)
}

fn parse_entries(v: &JsValue) -> Vec<LibEntry> {
    let arr = js_sys::Array::from(v);
    let mut out = Vec::new();
    for item in arr.iter() {
        let g = |k: &str| js_sys::Reflect::get(&item, &JsValue::from_str(k)).unwrap_or(JsValue::NULL);
        out.push(LibEntry {
            id: g("id").as_f64().unwrap_or(0.0),
            name: g("name").as_string().unwrap_or_default(),
            title: g("title").as_string().unwrap_or_default(),
            key: g("key").as_string().unwrap_or_default(),
            tempo: g("tempo").as_f64().unwrap_or(0.0) as u16,
            style: g("style").as_string().unwrap_or_default(),
        });
    }
    out
}

pub type LibInbox = Rc<RefCell<Option<Vec<LibEntry>>>>;
pub type BytesInbox = Rc<RefCell<Option<(String, Vec<u8>)>>>;

fn push_list(into: LibInbox, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(v) = list().await {
            *into.borrow_mut() = Some(parse_entries(&v));
            ctx.request_repaint();
        }
    });
}

/// (Re)load the library list into `into`.
pub fn refresh(into: LibInbox, ctx: egui::Context) {
    push_list(into, ctx);
}

/// Fetch a song's bytes by id and drop them into the shared bytes inbox (the
/// same one the file picker uses), so loading is handled uniformly.
pub fn load(id: f64, name: String, into: BytesInbox, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(v) = get(id).await {
            let bytes = js_sys::Uint8Array::new(&v).to_vec();
            if !bytes.is_empty() {
                *into.borrow_mut() = Some((name, bytes));
                ctx.request_repaint();
            }
        }
    });
}

/// Fetch a song's bytes by id and trigger a browser download.
pub fn download(id: f64, name: String) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(v) = get(id).await {
            let bytes = js_sys::Uint8Array::new(&v).to_vec();
            if !bytes.is_empty() {
                download_file(&name, &bytes);
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub fn save_song(
    name: String,
    bytes: Vec<u8>,
    title: String,
    key: String,
    tempo: u16,
    style: String,
    refresh_into: LibInbox,
    ctx: egui::Context,
) {
    wasm_bindgen_futures::spawn_local(async move {
        let _ = save(&name, &bytes, &title, &key, tempo as f64, &style).await;
        if let Ok(v) = list().await {
            *refresh_into.borrow_mut() = Some(parse_entries(&v));
            ctx.request_repaint();
        }
    });
}

pub fn remove_song(id: f64, refresh_into: LibInbox, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        let _ = remove(id).await;
        if let Ok(v) = list().await {
            *refresh_into.borrow_mut() = Some(parse_entries(&v));
            ctx.request_repaint();
        }
    });
}

pub fn rename_song(id: f64, title: String, refresh_into: LibInbox, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        let _ = rename(id, &title).await;
        if let Ok(v) = list().await {
            *refresh_into.borrow_mut() = Some(parse_entries(&v));
            ctx.request_repaint();
        }
    });
}
