# Keyboard overlay, caret follow, and right-edge control rail

Date: 2026-11-17. Status: approved by the user ("proceed" on the
recommended options). Choices marked *(my call)* were not discussed.

## Why

On the Pixel 10 Pro in landscape, opening the keyboard shrank the whole PC
picture into the strip above it, so nothing was readable while typing. The
floating control button also ended up in the middle of the screen (it
picked its position while the phone was still portrait) and looked out of
place.

## Goals

* While the keyboard is open, the PC picture keeps its size; the keyboard
  covers the bottom of it, and the view slides so the place being typed
  into stays visible just above the keyboard.
* The place being typed into is the PC's **text caret** when the PC can
  find one, otherwise the pointer (where the user last clicked).
* Controls live in a slim bar on the right edge, auto-hide to a small tab,
  and never sit in the middle of the picture. A setting moves them left.

## Non-goals / anti-goals

* No custom phone keyboard (Gboard's prediction, swipe and languages are
  better than anything we would build).
* No rotation to portrait while typing (jarring).
* Caret lookup must never slow input or the stream: it runs on its own
  thread, only while the phone's keyboard is open.

## Approaches considered

1. Overlay + follow caret (chosen): best result; needs a PC-side caret
   finder and two protocol messages.
2. Overlay + follow pointer: phone-only; drifts when text wraps away from
   the click point. Kept as the fallback inside (1).
3. Portrait while typing: rejected (rotation every time).

## Design

**Protocol v2** (v1 still accepted both ways):
* `TextInput { active }` C→H, tag 23. Sent only when v2 was negotiated.
* `Caret { visible, x, y, w, h }` H→C, tag 24. Normalised 0..65535 to the
  streamed display (same space as pointer positions); `visible=false` when
  no caret is found or it is on another display. Sent only on change,
  only while text input is active.

**Host caret finder** (`input-windows::caret`), polled every 100 ms on a
dedicated thread with COM (MTA) while text input is active:
1. `GetGUIThreadInfo` of the foreground thread: classic Win32 carets.
2. UI Automation: focused element → `TextPattern2::GetCaretRange` (falls
   back to the selection) → bounding rectangle, expanded to one character
   when the caret range is empty. UIA timeouts are set short *(my call:
   300 ms)* so a hung app cannot stall the thread.
3. Otherwise: no caret (the phone falls back to the pointer).

**Phone**
* The remote view no longer shrinks for the keyboard; only the shortcut
  row rides above it. The view knows the keyboard height and treats the
  area above it as the visible region.
* When the keyboard opens, and whenever the caret moves while it is open,
  pan (and if needed zoom in to at least 1.5× *(my call)*) so the caret
  sits in the lower third of the visible region. Without a caret, use the
  pointer.
* Control rail: right edge, inside the display cutout insets, buttons
  keyboard, trackpad/touch, zoom (cycles Fit → 2× → 1:1), more (panel with
  quality, display, stats, diagnostics), disconnect. Auto-hides after 4 s
  without use to a thin tab; tap the tab to bring it back. Setting:
  "Controls on the left".

## Left to the builder

Exact sizes, animation, and how the caret rectangle is clamped near the
edges.
