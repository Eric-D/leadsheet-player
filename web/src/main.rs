//! Entry point. The real application is browser-only (wasm32). Decoding lives
//! in the `leadsheet` crate; this binary is just the egui/Web-Audio front-end.

#[cfg(target_arch = "wasm32")]
mod app;
#[cfg(target_arch = "wasm32")]
mod audio;
#[cfg(target_arch = "wasm32")]
mod cloud;
#[cfg(target_arch = "wasm32")]
mod library;
#[cfg(target_arch = "wasm32")]
mod notation;

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    console_error_panic_hook::set_once();
    let _ = eframe::WebLogger::init(log::LevelFilter::Warn);

    let web_options = eframe::WebOptions::default();
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("no window")
            .document()
            .expect("no document");
        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("missing #the_canvas_id")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("#the_canvas_id is not a canvas");

        eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
            )
            .await
            .expect("failed to start eframe");
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!(
        "biab-web is a WebAssembly application.\n\
         Build it with:  trunk serve  (then open the printed URL)\n\
         Validate the parser on the host with:  cargo run --example validate"
    );
}
