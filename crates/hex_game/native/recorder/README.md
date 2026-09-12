# Native battle recorder

`build.rs` compiles `Recorder.swift` only for macOS builds with `arena-prototype`.
An Apple SDK exposing macOS 15 ScreenCaptureKit is required. Cargo maps Rust's
`aarch64` / `x86_64` target to Swift's `arm64` / `x86_64`, embeds the Info.plist,
ad-hoc signs the helper, and embeds its bytes into the game. Existing release
staging therefore includes the helper in each Mac game executable automatically;
no Python, ffmpeg or additional user installation is needed. The game remains
usable before macOS 15, with recording unavailable.

The explicit Record action installs these embedded bytes at the stable path
`~/Library/Application Support/Hex Game/Recorder/Hex Game Recorder.app` and starts
the helper through piped JSON lines. Native screen authorization belongs to
**Hex Game Recorder**. Changing an unsigned development build can require renewed
macOS authorization. Production signing/notarization remains the release owner's
existing responsibility; this is a local development implementation.

Only the requesting game's PID and its single exact native window title can
resolve a source. The helper retains the resulting window ID and never broadens
to a display, another process or child window. Bevy menus/HUD drawn inside that
window are included. Capture is SDR H.264 MP4, at most 1920×1080, at most 30 fps,
with a three-frame native queue and audio/microphone both disabled. Recording
continues while the game pauses, dies or restarts. Finalization precedes normal
quit/window closure. Source/process loss, disk pressure and startup/finalization
timeouts produce explicit events; incomplete files keep `.partial.mp4` names.

Final MP4s and accompanying `.events.jsonl` files go under
`~/Movies/Hex Game/Recordings`. Events record build/package identity, settings,
native lifecycle, reset generation, pause/death/restart and F9 bookmarks. A clip
is successful only after the native finished callback, a nonempty output check
and successful metadata flush. No frame buffers pass through Rust or PNG encoding.

Safe validation without recording:

```sh
xcrun swiftc -O -swift-version 5 -warnings-as-errors -parse-as-library \
  -target arm64-apple-macos12.3 Recorder.swift -o /tmp/hex-game-recorder-test \
  -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker Info.plist
/tmp/hex-game-recorder-test --self-test
```

The self-test verifies sizing and JSON decoding before initializing AppKit or
requesting any capture permission. Protocol tests may send `quit`, malformed JSON
or invalid start parameters; valid recording tests require an explicit native
playtest. Required integration wiring is `mod recording`, `recording::install`,
`WindowPlugin.close_when_requested = false`, and menu Quit through
`Recorder::request_quit()`. The recording system runs after Tick and before Present.

Native acceptance still needs permission denial/grant, actual MP4 playback,
pause/death/restart continuity, bookmarks, window resizing/fullscreen/minimizing,
normal quit and recording on/off frame-time comparison. This README and the safe
protocol tests do not claim those live behaviors were observed.
