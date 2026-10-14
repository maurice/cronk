//! Fake-terminal setup and input. Helpers do not implicitly settle the UI:
//! each suite chooses its update order and number of rounds explicitly.

use cronk::{config::Config, demo, gitlab::GitLab, ui::Cronk};
use std::path::Path;
use tui_lipan::{TestBackend, core::event::MouseKind, prelude::*};

pub type Ui = TestBackend<Cronk>;

/// Deliberate light/dark palette sample for expensive visual interaction tests.
/// This is a subset, not a second catalogue of all supported themes.
pub const REPRESENTATIVE_THEMES: [&str; 3] = ["midnight", "dracula", "light"];

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

/// Mount a demo workspace with manual focus, without extra update/draw rounds.
/// A supplied config path is saved only if absent: creating a test workspace
/// is a deliberate config save, not a navigation event.
/// The underlying TestBackend constructor initializes and draws its first frame.
pub fn mount_demo(config: Config, path: Option<&Path>, viewport: Rect) -> Ui {
    if let Some(path) = path.filter(|path| !path.exists()) {
        config.save(path).unwrap();
    }
    mount_at(config, path, None, true, viewport)
}

/// Mount at an explicit initial viewport with manual focus. Unlike mount_demo,
/// this does not save a missing config: onboarding and migration tests must
/// retain control of workspace creation. The application can still initialize
/// its own storage normally; this helper does not suppress those side effects.
pub fn mount_at(
    config: Config,
    path: Option<&Path>,
    api: Option<GitLab>,
    demo: bool,
    viewport: Rect,
) -> Ui {
    TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        application(config, path, api, demo),
        (),
        viewport,
    )
}

/// Mount using the framework's default initial viewport and manual focus.
/// Resizing afterwards is intentionally distinct from mount_at: initial layout
/// can update state and enqueue viewport callbacks before the resize.
pub fn mount_default(config: Config, path: Option<&Path>, api: Option<GitLab>, demo: bool) -> Ui {
    mount_default_with_app(
        App::new().focus_policy(FocusPolicy::Manual),
        config,
        path,
        api,
        demo,
    )
}

/// Preserve a caller's app options, including its focus policy, at the default
/// initial viewport. Does not save a missing config or perform extra rounds.
pub fn mount_default_with_app(
    app: App,
    config: Config,
    path: Option<&Path>,
    api: Option<GitLab>,
    demo: bool,
) -> Ui {
    TestBackend::new_with_app(app, application(config, path, api, demo), ())
}

fn application(config: Config, path: Option<&Path>, api: Option<GitLab>, demo: bool) -> Cronk {
    Cronk {
        config,
        path: path.map(Path::to_path_buf),
        api,
        demo,
    }
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

pub fn type_text(ui: &mut Ui, text: &str) {
    for character in text.chars() {
        key(ui, KeyCode::Char(character));
    }
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
        .unwrap_or_else(|| panic!("missing element {key:?}:\n{}", frame_text(ui)))
}
