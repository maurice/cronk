use cronk::{
    config::Config,
    demo,
    ui::{Action, Cronk, DialogKind, Msg},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    time::Duration,
};
use tui_lipan::{TestBackend, prelude::*};

type Ui = TestBackend<Cronk>;
fn mount(config: Config, path: &Path, demo: bool) -> Ui {
    let mut ui = TestBackend::new_with_app_and_viewport(
        App::new().focus_policy(FocusPolicy::Manual),
        Cronk {
            config,
            path: Some(path.to_owned()),
            api: None,
            demo,
        },
        (),
        Rect {
            x: 0,
            y: 0,
            w: 110,
            h: 38,
        },
    );
    settle(&mut ui);
    ui
}
fn settle(ui: &mut Ui) {
    for _ in 0..3 {
        ui.pump().unwrap();
        ui.render();
    }
}
fn send(ui: &mut Ui, msg: Msg) {
    ui.dispatch(msg).unwrap();
    settle(ui);
}
fn key(ui: &mut Ui, code: KeyCode, mods: KeyMods) {
    ui.send_key(KeyEvent { code, mods }).unwrap();
    settle(ui);
}
fn replace(ui: &mut Ui, field: usize, text: &str) {
    send(ui, Msg::DialogField(field));
    key(ui, KeyCode::Char('a'), KeyMods::CTRL);
    ui.send_paste(text).unwrap();
    settle(ui);
}
fn validate(ui: &mut Ui) {
    ui.advance(Duration::from_millis(450));
    for _ in 0..200 {
        settle(ui);
        if ui.state().onboarding.validated.is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("validation did not finish");
}

#[test]
fn first_run_demo_validates_multiple_projects_and_saves_header_and_dismissal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.demo.toml");
    let mut ui = mount(Config::default(), &path, true);
    assert!(matches!(
        ui.state().dialog.as_ref().unwrap().kind,
        DialogKind::Onboarding
    ));
    assert!(
        ui.state()
            .dialog
            .as_ref()
            .unwrap()
            .help
            .contains(path.to_str().unwrap())
    );
    assert!(!path.exists());
    replace(&mut ui, 2, "9001, 9002, 9001");
    validate(&mut ui);
    assert!(ui.state().onboarding.validated.as_ref().unwrap().valid);
    assert!(ui.state().onboarding.feedback[2].contains("2 project(s) verified"));
    send(&mut ui, Msg::Submit);
    assert!(ui.state().dialog.is_none());
    let saved = Config::load(&path).unwrap();
    assert!(!saved.onboarding);
    assert_eq!(saved.projects.len(), 2);
    let source = std::fs::read_to_string(&path).unwrap();
    assert!(source.starts_with("# Cronk configuration — https://github.com/maurice/cronk\n"));
    assert!(source.contains("restart"));
    assert!(mount(saved, &path, true).state().dialog.is_none());
}

#[test]
fn invalid_fields_never_save_and_skip_discards_drafts_and_pending_checks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.demo.toml");
    let mut ui = mount(Config::default(), &path, true);
    replace(&mut ui, 0, "glpat-do-not-store-me");
    replace(&mut ui, 1, "invalid-url");
    validate(&mut ui);
    assert!(!ui.state().onboarding.validated.as_ref().unwrap().valid);
    assert!(ui.state().onboarding.feedback[0].contains("variable name"));
    assert!(ui.state().onboarding.feedback[1].contains("absolute"));
    assert!(ui.state().onboarding.feedback[2].contains("at least one"));
    send(&mut ui, Msg::Submit);
    assert!(!path.exists());
    replace(&mut ui, 2, "9001");
    assert!(ui.state().onboarding.validated.is_none());
    send(&mut ui, Msg::CloseDialog);
    ui.advance(Duration::from_secs(1));
    settle(&mut ui);
    let saved = Config::load(&path).unwrap();
    assert!(!saved.onboarding);
    assert_eq!(saved.token_env, "GITLAB_TOKEN");
    assert_eq!(saved.gitlab_url, "https://gitlab.com");
    assert!(saved.projects.is_empty());
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("glpat-do-not-store-me")
    );
    assert!(ui.state().dialog.is_none());
    send(&mut ui, Msg::Action(Action::Onboarding));
    assert!(ui.state().dialog.is_some());
}

#[test]
fn edits_invalidate_success_and_unknown_demo_projects_have_feedback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.demo.toml");
    let mut ui = mount(
        Config {
            projects: demo::projects(),
            ..Config::default()
        },
        &path,
        true,
    );
    validate(&mut ui);
    assert!(ui.state().onboarding.validated.as_ref().unwrap().valid);
    let epoch = ui.state().onboarding.epoch;
    let stale = ui.state_mut().onboarding.validated.take().unwrap();
    replace(&mut ui, 2, "unknown/project");
    send(&mut ui, Msg::SetupValidated(epoch, Box::new(stale)));
    assert!(ui.state().onboarding.validated.is_none());
    send(&mut ui, Msg::Submit);
    assert!(!path.exists());
    validate(&mut ui);
    assert!(ui.state().onboarding.feedback[2].contains("fictional project"));
    assert!(!ui.state().onboarding.validated.as_ref().unwrap().valid);
}

#[test]
fn save_and_skip_failures_retain_onboarding() {
    let dir = tempfile::tempdir().unwrap();
    let mut ui = mount(
        Config {
            projects: demo::projects(),
            ..Config::default()
        },
        dir.path(),
        true,
    );
    validate(&mut ui);
    send(&mut ui, Msg::Submit);
    assert!(ui.state().dialog.is_some());
    assert!(ui.state().config.onboarding);
    assert!(ui.state().onboarding.validated.is_some());
    send(&mut ui, Msg::CloseDialog);
    assert!(ui.state().dialog.is_some());
    assert!(ui.state().config.onboarding);
}

#[test]
fn live_first_run_without_credentials_is_not_mistaken_for_demo() {
    let dir = tempfile::tempdir().unwrap();
    let mut ui = mount(
        Config {
            token_env: "CRONK_ONBOARDING_TEST_UNSET_TOKEN".into(),
            ..Config::default()
        },
        &dir.path().join("config.toml"),
        false,
    );
    assert!(!ui.state().demo);
    assert!(ui.state().items.is_empty());
    validate(&mut ui);
    assert!(ui.state().onboarding.feedback[0].contains("unset or empty"));
    send(&mut ui, Msg::CloseDialog);
    assert!(!ui.state().config.onboarding);
}

#[test]
fn live_validation_authenticates_and_resolves_a_project_asynchronously() {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let host = format!("http://{}", server.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        for expected in ["/api/v4/user", "/api/v4/projects/team%2Frepo"] {
            let (mut stream, _) = server.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = vec![];
            loop {
                let mut bytes = [0; 1024];
                let n = stream.read(&mut bytes).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&bytes[..n]);
                if request.windows(4).any(|b| b == b"\r\n\r\n") {
                    break;
                }
            }
            assert!(String::from_utf8_lossy(&request).starts_with(&format!("GET {expected} ")));
            let body = if expected.ends_with("/user") {
                r#"{"id":42,"username":"tester","name":"Test User"}"#
            } else {
                r#"{"id":7,"name":"repo","path_with_namespace":"team/repo"}"#
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let dir = tempfile::tempdir().unwrap();
    // PATH is present and header-safe; the local fixture ignores its value.
    // No environment mutation or real credential is needed for this test.
    let mut ui = mount(
        Config {
            token_env: "PATH".into(),
            gitlab_url: host,
            ..Config::default()
        },
        &dir.path().join("config.toml"),
        false,
    );
    replace(&mut ui, 2, "team/repo");
    validate(&mut ui);
    worker.join().unwrap();
    let validation = ui.state().onboarding.validated.as_ref().unwrap();
    assert!(validation.valid, "{:?}", validation.feedback);
    assert_eq!(validation.user.username, "tester");
    assert_eq!(validation.config.projects[0].id, 7);
    assert!(validation.api.is_some());
}
