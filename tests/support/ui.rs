//! Fake-terminal setup and input. Helpers do not implicitly settle the UI:
//! each suite chooses its update order and number of rounds explicitly.

use cronk::{config::Config, demo, ui::Cronk};
use std::path::Path;
use tui_lipan::{TestBackend, core::event::MouseKind, prelude::*};

pub type Ui = TestBackend<Cronk>;

pub fn demo_config() -> Config {
    Config {
        projects: demo::projects(),
        onboarding: false,
        ..Config::default()
    }
}

pub fn viewport(width: u16, height: u16) -> Rect {
    Rect {
        x: 0,
        y: 0,
        w: width,
        h: height,
    }
}

/// Mount a demo workspace with manual focus, without pumping or rendering.
/// A supplied config path is saved only if absent: creating a test workspace
/// is a deliberate config save, not a navigation event.
pub fn mount_demo(config: Config, path: Option<&Path>, viewport: Rect) -> Ui {
    if let Some(path) = path.filter(|path| !path.exists()) {
        config.save(path).unwrap();
    }
    TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config,
            path: path.map(Path::to_path_buf),
            api: None,
            demo: true,
        },
        (),
        viewport,
    )
}

/// Process queued updates, then draw. The last draw can queue viewport callbacks.
pub fn pump_render(ui: &mut Ui, rounds: usize) {
    for _ in 0..rounds {
        ui.pump().unwrap();
        ui.render();
    }
}

/// Draw, then process updates, including callbacks queued by that draw.
/// The last updates may require another draw before inspecting the screen.
pub fn render_pump(ui: &mut Ui, rounds: usize) {
    for _ in 0..rounds {
        ui.render();
        ui.pump().unwrap();
    }
}

pub fn key(ui: &mut Ui, code: KeyCode) {
    modified_key(ui, code, KeyMods::NONE);
}

pub fn modified_key(ui: &mut Ui, code: KeyCode, mods: KeyMods) {
    ui.send_key(KeyEvent { code, mods }).unwrap();
}

pub fn mouse(ui: &mut Ui, x: u16, y: u16, kind: MouseKind) {
    ui.send_mouse(MouseEvent {
        x,
        y,
        kind,
        mods: KeyMods::NONE,
    })
    .unwrap();
}

/// Read the last rendered frame without drawing or draining updates.
pub fn frame_text(ui: &Ui) -> String {
    ui.capture_frame().plain_text()
}

/// Draw a fresh frame before reading it; does not drain resulting callbacks.
pub fn render_text(ui: &mut Ui) -> String {
    ui.render();
    frame_text(ui)
}

pub fn element_rect(ui: &Ui, key: &str) -> Rect {
    ui.rect_of_key(&key.to_owned().into())
        .unwrap_or_else(|| panic!("missing element {key}"))
}
