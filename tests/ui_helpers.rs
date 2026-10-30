//! Plain unit-style contract tests of the shared fake-terminal helpers.
//! Functional UI stories live in the feature suites, not in this file.

mod support;

use cronk::{config::Config, ui::DialogKind};
use std::{collections::HashSet, panic::AssertUnwindSafe};
use support::ui;
use tui_lipan::{
    core::event::{MouseButton, MouseKind},
    prelude::*,
};

#[test]
fn demo_configuration_and_representative_themes_are_valid() {
    let config = ui::demo_config();
    assert!(!config.onboarding);
    assert!(!config.projects.is_empty());
    config.validate().unwrap();
    assert_eq!(
        ui::REPRESENTATIVE_THEMES
            .into_iter()
            .collect::<HashSet<_>>()
            .len(),
        ui::REPRESENTATIVE_THEMES.len()
    );
    for theme in ui::REPRESENTATIVE_THEMES {
        assert!(cronk::config::THEMES.contains(&theme));
        Config {
            theme: theme.into(),
            ..config.clone()
        }
        .validate()
        .unwrap();
    }
}

#[test]
fn raw_mount_keeps_onboarding_in_control_of_config_creation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("workspace.toml");
    let ui = ui::mount_at(Config::default(), Some(&path), true, ui::viewport(110, 38));
    assert!(!path.exists());
    assert_eq!(ui.viewport(), ui::viewport(110, 38));
    assert!(matches!(
        ui.state().dialog.as_ref().unwrap().kind,
        DialogKind::Onboarding
    ));
}

#[test]
fn demo_mount_saves_a_missing_workspace_deliberately() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("workspace.toml");
    let mut config = ui::demo_config();
    config.theme = "dracula".into();
    let ui = ui::mount_demo(config.clone(), Some(&path), ui::viewport(80, 24));
    assert!(path.exists());
    assert_eq!(Config::load(&path).unwrap().theme, config.theme);
    assert_eq!(ui.state().config.theme, config.theme);
    assert!(ui.state().dialog.is_none());
}

#[test]
fn demo_mount_does_not_replace_an_existing_config() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("workspace.toml");
    let mut config = ui::demo_config();
    config.theme = "light".into();
    config.save(&path).unwrap();
    let original = std::fs::read(&path).unwrap();
    config.theme = "dracula".into();
    let ui = ui::mount_demo(config, Some(&path), ui::viewport(80, 24));
    assert_eq!(ui.state().config.theme, "dracula");
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn default_mount_can_be_resized_after_its_initial_frame() {
    let mut ui = ui::mount_default(ui::demo_config(), None, true);
    let initial = ui.viewport();
    let resized = ui::viewport(initial.w + 10, initial.h + 10);
    ui.set_viewport(resized);
    ui::pump_render(&mut ui, 3);
    assert_eq!(ui.viewport(), resized);
    assert_eq!(ui.capture_frame().width, resized.w);
    assert_eq!(ui.capture_frame().height, resized.h);
}

#[test]
fn keyboard_helpers_deliver_modifiers_and_unicode_text() {
    let mut ui = ui::mount_demo(ui::demo_config(), None, ui::viewport(100, 30));
    ui::modified_key(&mut ui, KeyCode::Char('p'), KeyMods::CTRL);
    assert!(matches!(
        ui.state().dialog.as_ref().unwrap().kind,
        DialogKind::Commands
    ));
    ui::type_text(&mut ui, "starλ");
    assert_eq!(
        ui.state().dialog.as_ref().unwrap().fields[0].value(),
        "starλ"
    );
    ui::key(&mut ui, KeyCode::Backspace);
    assert_eq!(
        ui.state().dialog.as_ref().unwrap().fields[0].value(),
        "star"
    );
}

#[test]
fn mouse_and_element_helpers_deliver_clicks_to_the_requested_tab() {
    let mut ui = ui::mount_demo(ui::demo_config(), None, ui::viewport(100, 30));
    let rect = ui::element_rect(&ui, "workspace-tab-1");
    let x = rect.x as u16 + rect.w / 2;
    let y = rect.y as u16 + rect.h / 2;
    ui::mouse(&mut ui, x, y, MouseKind::Down(MouseButton::Left));
    ui::mouse(&mut ui, x, y, MouseKind::Up(MouseButton::Left));
    ui::pump_render(&mut ui, 3);
    assert_eq!(ui.state().config.active_tab, 1);
}

#[test]
fn frame_text_does_not_render_but_render_text_does() {
    let config = Config {
        active_tab: 2,
        ..ui::demo_config()
    };
    let mut ui = ui::mount_demo(config, None, ui::viewport(100, 30));
    let before = ui::frame_text(&ui);
    ui.state_mut().items.clear();
    assert_eq!(ui::frame_text(&ui), before);
    let after = ui::render_text(&mut ui);
    assert_ne!(after, before);
    assert!(after.contains("Nothing needs your attention"), "{after}");
}

#[test]
fn missing_element_reports_its_key_and_the_last_frame() {
    let ui = ui::mount_demo(ui::demo_config(), None, ui::viewport(100, 30));
    let frame = ui::frame_text(&ui);
    let error = std::panic::catch_unwind(AssertUnwindSafe(|| ui::element_rect(&ui, "missing-key")))
        .unwrap_err();
    let message = error.downcast::<String>().unwrap();
    assert!(message.contains("missing-key"));
    assert!(message.contains(&frame));
}
