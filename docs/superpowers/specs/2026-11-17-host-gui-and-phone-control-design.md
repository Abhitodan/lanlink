# Host GUI and phone control — design

Date: 2026-11-17. Status: proceeding on the author's calls (the user said
"proceed" without choosing; every choice below is marked as such and can be
reversed).

## Why

The first real-phone run (Pixel 10 Pro) worked, but controlling a
2560×1600 desktop on a 6-inch screen is hard: the whole desktop is fitted
into the phone, so in portrait it shows at about half size and targets are
tiny. The PC side is a terminal window, which is fine for development and
not for anyone else.

Two independent projects, in this order:

1. **Phone control** (felt on every use).
2. **Windows host app** replacing the console.

## Goals / non-goals / anti-goals

Goals
* Small targets on the PC can be hit reliably from the phone, one-handed
  where possible, without the user having to manage zoom by hand.
* The PC side is a normal Windows app: tray icon, window with pairing QR,
  approvals, paired devices, live status, emergency stop, a visible
  "being controlled" indicator.

Non-goals (now): changing the PC's resolution, clipboard/audio, installer
signing, iOS.

Anti-goals: anything that adds latency to the input path (animations run
on the phone only and never delay sending input); hidden control (the PC
must always show that it is being controlled); the overlay must not appear
in the stream the phone sees.

## Phone control

Approaches considered
* **A. Better defaults + camera that follows the pointer** (chosen): the
  plumbing exists (zoom/pan viewport, trackpad mode, local cursor). Use
  landscape, start zoomed in trackpad mode, keep the pointer in view with a
  smooth dead-zone camera, add a magnifier for direct touch, replace the
  top bar with a floating control button.
* B. Ask the PC for a phone-sized virtual resolution: biggest readability
  gain but invasive (changes the user's real desktop layout) — rejected as
  a default.
* C. Separate "touchpad screen" (phone as a pure trackpad, video on the
  PC only): useful but a different product (remote-mouse) — later, as a mode.

Design (author's calls)
1. **Landscape while controlling** (setting, default on): the PC is
   landscape; this alone roughly doubles the picture size.
2. **Laptop mode is the default for new installs**: trackpad pointer,
   view starts at 2× zoom, **follow camera** — the pointer may move freely
   inside a central dead zone (60 % of the view); beyond it the view glides
   to keep the pointer in that zone (eased, phone-side only).
3. **Precision curve**: slow finger motion moves the pointer less than
   1 PC pixel per phone pixel; fast flicks accelerate. Precision gain is
   independent of zoom.
4. **Direct touch keeps working**, plus a **magnifier loupe** (Android
   `Magnifier`, works on `SurfaceView` since API 29) above the finger while
   hovering or dragging, so the user sees exactly what is under the finger.
5. **Zoom presets**: Fit, 2×, and "1:1" (one PC pixel per phone pixel), plus
   double-tap with two fingers to toggle Fit ↔ last zoom.
6. **Floating control button** (draggable, snaps to edges, semi-transparent)
   opens a panel: keyboard, mouse mode, zoom presets, display, quality,
   stats, disconnect. Replaces the auto-hiding top bar that covered the
   PC's title bars.
7. **Three-finger tap** toggles the keyboard. Light haptic tick on clicks.

What is left to the builder: exact easing constants, icon art, panel layout.

## Windows host app

Approaches considered
* **A. egui/eframe + tray-icon + a Win32 overlay** (chosen): pure Rust, one
  small exe, MIT/Apache licences, links the existing `session-manager`
  directly — the GUI is a front end over `HostHandle` commands and events
  that already exist.
* B. Tauri (WebView2 + web UI): richer styling, but a web stack and a second
  language for the UI, close to the spirit of the brief's "no Electron".
* C. Slint: polished declarative UI, but its licence choice (GPL / royalty-
  free with attribution / commercial) needs a decision ADR 0010 hasn't made.

Design (author's calls)
* **Window** (dark, calm, one accent colour):
  - *Home*: status hero — "Ready for phones" / "Pixel 10 Pro is controlling
    this PC" — with a big **Stop** button; live tiles (fps, RTT, bitrate,
    encode ms) with sparklines.
  - *Pair*: big QR in the window, the 8-digit code, countdown ring; the
    approval prompt shows the 6-digit code large with Approve/Deny.
  - *Devices*: paired phones with last-seen, Revoke.
  - *Settings*: computer name, ask before every session, start at login,
    port (read-only display), open logs folder.
* **Tray icon**: neutral when idle, accent when a phone is connected, red
  dot while controlled. Menu: Open, Pair a phone, Stop session, Quit.
  Closing the window keeps the host running in the tray.
* **Being-controlled indicator**: a thin accent border around the captured
  display plus a small pill "Controlled by <phone> — Stop", topmost,
  click-through except the Stop pill, **excluded from capture**
  (`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`) so the phone never
  sees it.
* **Emergency stop hotkey**: Ctrl+Alt+Shift+S ends the session from
  anywhere (registered with `RegisterHotKey`; if taken, the app says so).
* Start at login: per-user `Run` key, off by default, toggled only by the
  user in Settings.

What is left to the builder: exact palette, sparkline window length, tile
layout at small window sizes.
