# Reproduce the syncing preview

The preview uses fictional demo data and simulated pending requests, without contacting GitLab.

```sh
cargo run --locked --example sync_preview
ffmpeg -v error \
  -framerate 20 -i target/sync-preview/midnight/%03d.png \
  -framerate 20 -i target/sync-preview/light/%03d.png \
  -filter_complex '[0:v]crop=1280:64:0:704[a];[1:v]crop=1280:64:0:704[b];[a][b]vstack,split[s0][s1];[s0]palettegen=stats_mode=full[p];[s1][p]paletteuse=dither=none' \
  -loop 0 -y docs/syncing-status.gif
```

The example asserts that both the dot colors and encoded PNGs change throughout the five-second cycle. It uses `TestBackend::capture_frame()`, which binds the advancing virtual clock like the live renderer. In tui-lipan 0.18.2, `capture_ui_snapshot()` does not bind that clock, so time-based custom effects appear frozen at time zero even if virtual time advances.

Check the **decoded GIF**, not just the source frames, to catch encoding errors:

```sh
ffmpeg -v error -i docs/syncing-status.gif -f framemd5 - \
  | awk '!/^#/ {print $NF}' | sort -u | wc -l
ffprobe -v error -show_entries format=duration -of csv=p=0 docs/syncing-status.gif
```

Expect more than 80 distinct decoded frames and a duration of `5.000000` seconds.
