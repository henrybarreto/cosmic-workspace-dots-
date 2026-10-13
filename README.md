# COSMIC Workspace Dots 1.0

Panel applet showing workspaces as circles, or squares when the COSMIC theme uses square corners (from the theme corner radius). The active workspace uses the accent color; click a dot to switch. With several outputs, each workspace group is shown with a wider gap between groups. The watcher reconnects automatically if the compositor connection drops.

## Install

```sh
cargo build --release
install -Dm755 target/release/cosmic-workspace-dots ~/.local/bin/cosmic-workspace-dots
install -Dm644 resources/com.github.henry.cosmic-workspace-dots.desktop \
  ~/.local/share/applications/com.github.henry.cosmic-workspace-dots.desktop
```

Make sure `~/.local/bin` is in your `PATH`, then add `COSMIC Workspace Dots` to a panel in the panel settings.

## Update

Rebuild and reinstall the binary as above, then restart the panel so it launches the new binary:

```sh
pkill -x cosmic-panel
```

## Settings

Right-click the applet to open a popup with a slider for the dot size (10–80% of the panel icon size, default 40%). It is saved with cosmic-config under `~/.config/cosmic/com.github.henry.cosmic-workspace-dots/v1/dot_percent`.
