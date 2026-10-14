# UI screenshot regression fixtures

`tests/ui_screenshots.rs` renders the dashboard, projects list, and command palette at a fixed 100×24 viewport and compares their PNGs with the committed fixtures here. The renderer uses DejaVu Sans Mono and masks only the commit-specific build revision in the header.

After intentionally changing a screen, regenerate with:

```sh
CRONK_UPDATE_UI_SNAPSHOTS=1 just test --test ui_screenshots
```

Review the PNG diffs before committing them; do not set the update variable in CI.
