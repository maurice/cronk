use cronk::{
    config::Config,
    demo,
    model::{ItemKey, ItemKind},
    ui::Cronk,
};
use tui_lipan::{TestBackend, prelude::*};

fn mount(kind: ItemKind, section: usize, theme: &str) -> TestBackend<Cronk> {
    let mut ui = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config: Config {
                projects: demo::projects(),
                active_tab: if kind == ItemKind::Issue { 2 } else { 3 },
                route: Some(ItemKey {
                    project: 9001,
                    iid: if kind == ItemKind::Issue { 1 } else { 102 },
                    kind,
                }),
                section: Some(section),
                theme: theme.into(),
                onboarding: false,
                ..Config::default()
            },
            path: None,
            api: None,
            demo: true,
        },
        (),
        Rect {
            x: 0,
            y: 0,
            w: 80,
            h: 60,
        },
    );
    settle(&mut ui);
    ui
}

fn settle(ui: &mut TestBackend<Cronk>) {
    for _ in 0..3 {
        ui.pump().unwrap();
        ui.render();
    }
}

fn point(ui: &TestBackend<Cronk>, needle: &str) -> (u16, u16) {
    let lines = ui.capture_frame().to_lines();
    let (y, line) = lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.contains(needle))
        .unwrap_or_else(|| panic!("missing {needle}:\n{}", lines.join("\n")));
    (
        line[..line.find(needle).unwrap()].chars().count() as u16,
        y as u16,
    )
}

#[test]
fn web_urls_are_underlined_blue_and_wrap_in_fields_and_pipeline() {
    for theme in ["midnight", "light"] {
        for (kind, section) in [
            (ItemKind::Issue, 0),
            (ItemKind::MergeRequest, 0),
            (ItemKind::MergeRequest, 2),
        ] {
            let mut ui = mount(kind, section, theme);
            let url = "https://example.test/a/very/long/path/that/must/wrap/in/the/narrow/detail/pane/link-tail";
            let item = &mut ui.state_mut().details.as_mut().unwrap().item;
            if section == 2 {
                item.pipeline.as_mut().unwrap().web_url = url.into();
            } else {
                item.web_url = url.into();
            }
            settle(&mut ui);
            let (x, y) = point(&ui, "https://example.test");
            let (tail_x, tail_y) = point(&ui, "link-tail");
            assert!(tail_y > y, "long URLs must wrap");
            let frame = ui.capture_frame();
            let link = frame.cell(x, y);
            assert!(link.modifiers.underline.is_some());
            assert_eq!(frame.cell(tail_x, tail_y).fg, link.fg);
            assert!(frame.cell(tail_x, tail_y).modifiers.underline.is_some());
            let (label_x, label_y) = point(&ui, "Web URL");
            assert_ne!(frame.cell(label_x, label_y).fg, link.fg);
            assert!(frame.cell(label_x, label_y).modifiers.underline.is_none());
        }
    }
}

#[test]
fn missing_or_invalid_web_urls_remain_plain_text() {
    for url in [
        "",
        "not a URL",
        "file:///etc/passwd",
        "https://example.test/\nunsafe",
    ] {
        let mut ui = mount(ItemKind::Issue, 0, "midnight");
        ui.state_mut().details.as_mut().unwrap().item.web_url = url.into();
        settle(&mut ui);
        let (label_x, y) = point(&ui, "Web URL");
        let frame = ui.capture_frame();
        let value_x = label_x + "Target branch".len() as u16 + 2;
        assert!(frame.cell(value_x, y).modifiers.underline.is_none());
        if url.is_empty() {
            assert_eq!(frame.cell(value_x, y).symbol, "—");
        }
    }
}

// Run browser activation in a subprocess with a fake opener. Never mutate the
// parallel test runner's environment or launch the developer's actual browser.
#[cfg(target_os = "linux")]
#[test]
fn clicking_links_opens_the_full_destination() {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        process::Command,
        time::{Duration, Instant},
    };
    use tui_lipan::core::event::{MouseButton, MouseKind};

    const OUTPUT: &str = "CRONK_TEST_LINK_OUTPUT";
    let Ok(output) = std::env::var(OUTPUT) else {
        let dir = tempfile::tempdir().unwrap();
        let opener = dir.path().join("xdg-open");
        fs::write(
            &opener,
            "#!/bin/sh\nprintf '%s' \"$1\" > \"$CRONK_TEST_LINK_OUTPUT\"\n",
        )
        .unwrap();
        fs::set_permissions(&opener, fs::Permissions::from_mode(0o755)).unwrap();
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "clicking_links_opens_the_full_destination"])
            .env("PATH", dir.path())
            .env(OUTPUT, dir.path().join("opened-url"))
            .env_remove("WSL_DISTRO_NAME")
            .env_remove("WSL_INTEROP")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    };

    for (kind, section) in [
        (ItemKind::Issue, 0),
        (ItemKind::MergeRequest, 0),
        (ItemKind::MergeRequest, 2),
        (ItemKind::Issue, 1),
        (ItemKind::MergeRequest, 4),
    ] {
        let mut ui = mount(kind, section, "midnight");
        ui.set_viewport(Rect {
            x: 0,
            y: 0,
            w: 80,
            h: 120,
        });
        let url = "https://example.test/a/very/long/path/that/must/wrap/in/the/narrow/detail/pane/link-tail?query=value#fragment";
        let details = ui.state_mut().details.as_mut().unwrap();
        let needle = if section == 0 {
            details.item.web_url = url.into();
            "link-tail"
        } else if section == 2 {
            details.item.pipeline.as_mut().unwrap().web_url = url.into();
            "link-tail"
        } else if section == 1 {
            details.item.description = format!("[Open documentation]({url})");
            "Open documentation"
        } else {
            details.discussions[0].notes[0].body = format!("Read [Open documentation]({url}).");
            "Open documentation"
        };
        settle(&mut ui);
        let (x, y) = point(&ui, needle);
        let _ = fs::remove_file(&output);
        for kind in [
            MouseKind::Down(MouseButton::Left),
            MouseKind::Up(MouseButton::Left),
        ] {
            ui.send_mouse(MouseEvent {
                x,
                y,
                kind,
                mods: KeyMods::NONE,
            })
            .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if fs::read_to_string(&output).ok().as_deref() == Some(url) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "browser did not receive the full URL for {kind:?} section {section}; received {:?}",
                fs::read_to_string(&output)
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            ui.state().dialog.is_none(),
            "link clicks must not activate surrounding actions"
        );
    }
}
