# Changelog

## [2.5.0] - 2026-09-06

**Highlights:** This release brings a full Linux tray icon implementation (replacing the previous no-op stub) alongside a batch of fixes and improvements merged in from upstream, including a YAML config parsing regression fix and Linux packaging improvements.

### ✨ New Features
- **linux:** add real GNOME AppIndicator tray icon support via ksni (StatusNotifierItem/dbusmenu), reusing the same engine-driven menu pipeline macOS/Windows already use; includes a live systemd `--user` service status line and restart action in the tray menu, plus documentation of Wayland runtime prerequisites (evdev/uinput permissions via setcap or the input group)

### 🐛 Fixes
- **config:** parse unquoted special-char values in YAML flow collections, restoring pre-2.4.0 (serde_yaml) tolerance for values starting with `:`, `>`, `|` (#2761)

### 🛠 Improvements
- **linux:** install desktop file and icon in `.deb`/`.rpm` packages; renamed the icon file for AppImage/deb/rpm compatibility (#2757)

### 📚 Documentation
- Add inspect.software health badge to README (#2780)
