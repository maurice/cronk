//! Capture the five-second syncing cycle used in docs/syncing-status.gif.
//!
//! Run `cargo run --locked --example sync_preview`, then use the ffmpeg command
//! documented in docs/syncing-preview.md to combine the two themes.
use std::{collections::HashSet, time::Duration};

use cronk::{config::Config, demo, ui::Cronk};
use tui_lipan::{PngOptions, TestBackend, prelude::*};

fn main() -> anyhow::Result<()> {
    for theme in ["midnight", "light"] {
        let mut ui = TestBackend::new_with_app_and_viewport(
            App::new().focus_policy(FocusPolicy::Manual),
            Cronk {
                config: Config {
                    projects: demo::projects(),
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
                h: 24,
            },
        );
        for _ in 0..3 {
            ui.pump()?;
            ui.render();
        }
        // Fictional in-flight request, with no API or real network traffic.
        ui.state_mut().mutation_pending = true;
        ui.state_mut().status = format!("{theme} theme · Requests to GitLab are in progress");
        ui.render();
        let dot = ui.rect_of_key(&"sync-status".into()).expect("sync dot");
        let dir = format!("target/sync-preview/{theme}");
        std::fs::create_dir_all(&dir)?;
        let mut colors = HashSet::new();
        let mut images = HashSet::new();
        for index in 0..100 {
            // capture_ui_snapshot() does not bind the animation clock in lipan 0.18.2.
            // capture_frame() follows the live renderer and evaluates effects at virtual time.
            let frame = ui.capture_frame();
            let cell = frame.cell(dot.x as u16, dot.y as u16);
            assert_eq!(cell.symbol, "●");
            colors.insert(cell.fg.to_rgb().unwrap());
            let png = frame.to_png(&PngOptions::default())?;
            images.insert(png.clone());
            std::fs::write(format!("{dir}/{index:03}.png"), png)?;
            ui.advance_frame(Duration::from_millis(50));
        }
        anyhow::ensure!(
            colors.len() > 80,
            "{theme}: color sequence is static or undersampled"
        );
        anyhow::ensure!(
            images.len() > 80,
            "{theme}: PNG sequence is static or undersampled"
        );
        println!(
            "{theme}: {} distinct colors, {} distinct PNGs over five seconds",
            colors.len(),
            images.len()
        );
    }
    Ok(())
}
