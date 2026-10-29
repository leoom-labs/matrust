//! MatRust in the browser: one console session behind a small JSON API.
//! The contract (README.md) is shared with the website; keep it stable.
use matrust::{Array, Engine};
use std::sync::Once;
use wasm_bindgen::prelude::*;

/// Text output returned by one run; the page should not have to render more.
const MAX_OUTPUT: usize = 1024 * 1024;
const TRUNCATED: &str = "\n[Output truncated: a run returns at most 1 MB of text.]";

/// Most elements a vector shows in the VALUE column, as in the desktop.
const VECTOR_ELEMENTS: usize = 10;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = performance, js_name = now)]
    fn performance_now() -> f64;
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(message: &str);
}

/// A panic aborts the call; log its message first so the page can report it.
/// Rayon gets its one-thread pool here, before any kernel could ask for threads.
fn setup() {
    static SETUP: Once = Once::new();
    SETUP.call_once(|| {
        std::panic::set_hook(Box::new(|info| console_error(&format!("MatRust: {info}"))));
        if let Err(error) = matrust::initialize_worker_pool(1) {
            console_error(&format!("MatRust worker pool: {error}"));
        }
    });
}

/// One workspace with its figures, like one desktop Command Window.
#[wasm_bindgen]
pub struct Session {
    engine: Engine,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl Session {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Session {
        setup();
        Session {
            engine: Engine::default(),
        }
    }

    /// JSON `{"output", "error", "figures": [{"id", "svg"}], "ms"}`.
    pub fn run(&mut self, code: &str) -> String {
        let start = performance_now();
        let (lines, error) = match self.engine.execute(code) {
            Ok(execution) => (execution.output, None),
            // Statements before the error ran; their output is kept.
            Err(error) => (
                std::mem::take(&mut self.engine.failed_output.output),
                Some(error),
            ),
        };
        let figures = self.open_figures();
        serde_json::json!({
            "output": bounded_output(&lines),
            "error": error,
            "figures": figures,
            "ms": performance_now() - start,
        })
        .to_string()
    }

    /// Clear the workspace, figures and all engine state.
    pub fn reset(&mut self) {
        self.engine = Engine::default();
    }

    /// JSON `[{"name", "class", "size", "value"}]`, sorted by name.
    pub fn workspace(&self) -> String {
        let rows: Vec<_> = self
            .engine
            .workspace()
            .iter()
            .map(|(name, value)| {
                serde_json::json!({
                    "name": name,
                    "class": value.class(),
                    "size": format!("{}x{}", value.rows(), value.cols()),
                    "value": value_preview(value),
                })
            })
            .collect();
        serde_json::Value::Array(rows).to_string()
    }
}

impl Session {
    /// Light-theme SVG of every open figure, as the desktop keeps figure windows
    /// open across runs; the page redraws the whole strip after each run.
    fn open_figures(&self) -> Vec<serde_json::Value> {
        let mut figures: Vec<_> = self
            .engine
            .figures()
            .into_iter()
            .map(|(id, figure)| (id, figure.svg()))
            .collect();
        figures.sort_unstable_by_key(|(id, _)| *id);
        figures
            .into_iter()
            .map(|(id, svg)| serde_json::json!({ "id": id, "svg": svg }))
            .collect()
    }
}

/// MatRust version, as in the desktop About box.
#[wasm_bindgen]
pub fn version() -> String {
    matrust::VERSION.into()
}

/// Command Window text, cut at 1 MB on a character boundary.
fn bounded_output(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    if text.len() > MAX_OUTPUT {
        let mut end = MAX_OUTPUT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(TRUNCATED);
    }
    text
}

/// One real element as `format short` prints it alone (`16`, `0.8147`).
fn format_short(value: f64) -> String {
    matrust::display_format::disp(
        &matrust::Matrix::scalar(value).into(),
        &matrust::display_format::DisplayFormat::default(),
    )
    .ok()
    .flatten()
    .map_or_else(|| value.to_string(), |text| text.trim().to_owned())
}

/// The desktop Workspace VALUE column (src/desktop/workspace_row.rs): a
/// scalar's value, a vector of up to ten elements, otherwise `<rows>x<cols>
/// <class>`. Reads at most ten elements or 32 characters.
fn value_preview(value: &Array) -> String {
    fn list(items: impl Iterator<Item = String>) -> String {
        format!("[{}]", items.collect::<Vec<_>>().join(" "))
    }
    let vector =
        (value.rows() == 1 || value.cols() == 1) && (2..=VECTOR_ELEMENTS).contains(&value.len());
    match value {
        Array::Numeric(m) if m.len() == 1 => format_short(m.data()[0]),
        Array::Numeric(m) if vector => list(m.data().iter().map(|v| format_short(*v))),
        Array::Typed(t) if t.len() == 1 || vector => {
            let one = |i: usize| {
                if t.is_integer() {
                    t.int(i).to_string()
                } else {
                    format_short(t.real(i))
                }
            };
            if t.len() == 1 {
                one(0)
            } else {
                list((0..t.len()).map(one))
            }
        }
        Array::Logical(m) if m.len() == 1 => (m.data()[0] != 0).to_string(),
        Array::Logical(m) if vector => list(m.data().iter().map(|v| u8::from(*v != 0).to_string())),
        Array::Complex(m) if m.len() == 1 => {
            let z = m.data()[0];
            format!(
                "{}{}{}i",
                format_short(z.re),
                if z.im < 0. { "-" } else { "+" },
                format_short(z.im.abs())
            )
        }
        Array::Character(c) if c.rows() <= 1 => {
            let units = c.units();
            let text = String::from_utf16_lossy(&units[..units.len().min(32)]);
            format!("'{text}{}'", if units.len() > 32 { "..." } else { "" })
        }
        _ if value.rows() == 0 || value.cols() == 0 => "[]".into(),
        Array::Complex(_) => format!("{}x{} double (complex)", value.rows(), value.cols()),
        _ => format!("{}x{} {}", value.rows(), value.cols(), type_label(value)),
    }
}

fn type_label(value: &Array) -> &'static str {
    if value.is_complex() {
        "complex double"
    } else {
        value.class()
    }
}
