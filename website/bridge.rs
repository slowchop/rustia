mod garden;
mod playground;
mod race;

use wasm_bindgen::prelude::*;

// The worker collects one frame of drawing commands before updating the page.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = gardenOutput, js_name = sprite)]
    fn draw_sprite(shape: i32, x: f64, y: f64, angle: f64);
}

#[wasm_bindgen]
pub struct Session(garden::Garden);

#[wasm_bindgen]
impl Session {
    #[wasm_bindgen(constructor)]
    pub fn new(source: &str, radius: Option<f64>) -> Result<Session, String> {
        Self::with_drawing(source, include_str!("drawing.luau"), radius)
    }

    #[wasm_bindgen(js_name = withDrawing)]
    pub fn with_drawing(
        source: &str,
        drawing: &str,
        radius: Option<f64>,
    ) -> Result<Session, String> {
        Self::from_edit(source, Some(drawing.to_owned()), None, Vec::new(), radius)
    }

    #[wasm_bindgen(js_name = fromEdit)]
    pub fn from_edit(
        source: &str,
        drawing: Option<String>,
        flowerbed: Option<String>,
        previous: Vec<f64>,
        radius: Option<f64>,
    ) -> Result<Session, String> {
        garden::Garden::load(
            source,
            drawing.as_deref().unwrap_or(include_str!("drawing.luau")),
            flowerbed
                .as_deref()
                .unwrap_or(include_str!("flowerbed.luau")),
            previous,
            radius,
        )
        .map(Session)
        .map_err(|err| err.to_string())
    }

    #[wasm_bindgen(js_name = savedFlowers)]
    pub fn saved_flowers(&mut self) -> Result<Vec<f64>, String> {
        self.0.saved_flowers().map_err(|err| err.to_string())
    }

    #[wasm_bindgen(js_name = spriteProject)]
    pub fn sprite_project(
        names: Vec<String>,
        sources: Vec<String>,
        radius: f64,
    ) -> Result<Session, String> {
        let (main, resolver) =
            playground::project_sources(names, sources).map_err(|error| error.to_string())?;
        garden::Garden::load_with_host(
            &main,
            include_str!("drawing.luau"),
            include_str!("flowerbed.luau"),
            Vec::new(),
            Some(radius),
            Some(("main.luau", resolver)),
        )
        .map(Session)
        .map_err(|error| error.to_string())
    }

    #[wasm_bindgen(js_name = gardenProject)]
    pub fn garden_project(
        names: Vec<String>,
        sources: Vec<String>,
        previous: Vec<f64>,
        radius: f64,
    ) -> Result<Session, String> {
        let mut files: std::collections::BTreeMap<_, _> = names.into_iter().zip(sources).collect();
        let drawing = files.remove("drawing.luau").ok_or("Missing drawing.luau")?;
        let flowerbed = files
            .remove("flowerbed.luau")
            .ok_or("Missing flowerbed.luau")?;
        let (main, resolver) =
            playground::project_entry(files, "garden.luau").map_err(|error| error.to_string())?;
        garden::Garden::load_with_host(
            &main,
            &drawing,
            &flowerbed,
            previous,
            Some(radius),
            Some(("garden.luau", resolver)),
        )
        .map(Session)
        .map_err(|error| error.to_string())
    }

    pub fn animated(&self) -> bool {
        self.0.animated()
    }

    // The responsive garden supplies its open-space radius on every frame.
    #[wasm_bindgen(js_name = tickWithRadius)]
    pub fn tick_with_radius(
        &mut self,
        time: f64,
        mouse_x: Option<f64>,
        mouse_y: Option<f64>,
        radius: f64,
    ) -> Result<(), String> {
        self.0
            .tick(time, mouse_x.zip(mouse_y), Some(radius))
            .map_err(|err| err.to_string())
    }

    pub fn tick(
        &mut self,
        time: f64,
        mouse_x: Option<f64>,
        mouse_y: Option<f64>,
    ) -> Result<(), String> {
        self.0
            .tick(time, mouse_x.zip(mouse_y), None)
            .map_err(|err| err.to_string())
    }
}
