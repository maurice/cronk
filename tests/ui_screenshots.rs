use std::{fs, path::PathBuf};

use cronk::{
    config::Config,
    demo,
    ui::{Cronk, Msg},
};
use sha2::{Digest, Sha256};
use tui_lipan::{
    App, CapturedFrame, PngOptions, PngTextRenderer, TestBackend,
    prelude::{FocusPolicy, Rect},
};

type Ui = TestBackend<Cronk>;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 24;
const SNAPSHOT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/.snapshots/visual");

fn config(active_tab: usize) -> Config {
    Config {
        projects: demo::projects(),
        active_tab,
        theme: "midnight".into(),
        onboarding: false,
        animations: false,
        ..Config::default()
    }
}

fn mount(config: Config) -> Ui {
    let mut ui = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config,
            path: None,
            api: None,
            demo: true,
        },
        (),
        Rect {
            x: 0,
            y: 0,
            w: WIDTH,
            h: HEIGHT,
        },
    );
    ui.state_mut().tick = 12;
    ui.pump().unwrap();
    ui.render();
    ui
}

fn stable_png(ui: &mut Ui) -> Vec<u8> {
    let mut frame = ui.capture_frame();
    normalize_build_revision(&mut frame);

    let options = PngOptions {
        text_renderer: PngTextRenderer::Auto,
        font_family: Some("DejaVu Sans Mono".into()),
        render_cursor: false,
        ..PngOptions::default()
    };
    frame.to_png(&options).unwrap()
}

// The top-right build revision changes on every commit. Keep the rest of the header
// (version and layout) in the image, but normalize those 12 glyphs for a stable baseline.
fn normalize_build_revision(frame: &mut CapturedFrame) {
    let Some(line) = frame.to_lines().first().cloned() else {
        return;
    };
    let Some(prefix) = line.find("v0.0.0@") else {
        return;
    };
    let start = line[..prefix].chars().count() + "v0.0.0@".len();
    let row = usize::from(frame.width);
    for x in start..start.saturating_add(12).min(row) {
        frame.cells[x].symbol = "0".into();
    }
}

fn assert_screenshot(name: &str, ui: &mut Ui) {
    let actual = stable_png(ui);
    let path = PathBuf::from(SNAPSHOT_DIR).join(format!("{name}.png"));

    if std::env::var_os("CRONK_UPDATE_UI_SNAPSHOTS").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &actual).unwrap();
        return;
    }

    let expected = fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "missing {name} screenshot at {} ({error}); regenerate with CRONK_UPDATE_UI_SNAPSHOTS=1",
            path.display()
        )
    });
    let actual_sha = Sha256::digest(&actual);
    let expected_sha = Sha256::digest(&expected);
    assert_eq!(
        actual_sha[..],
        expected_sha[..],
        "{name} screenshot changed ({} bytes vs {}); inspect the UI and regenerate with CRONK_UPDATE_UI_SNAPSHOTS=1",
        actual.len(),
        expected.len(),
    );
}

#[test]
fn dashboard_screenshot_is_stable() {
    let mut ui = mount(config(0));
    assert_screenshot("dashboard", &mut ui);
}

#[test]
fn projects_screenshot_is_stable() {
    let mut ui = mount(config(1));
    assert_screenshot("projects", &mut ui);
}

#[test]
fn command_palette_screenshot_is_stable() {
    let mut ui = mount(config(2));
    ui.dispatch(Msg::Palette).unwrap();
    ui.pump().unwrap();
    ui.render();
    assert_screenshot("command-palette", &mut ui);
}
