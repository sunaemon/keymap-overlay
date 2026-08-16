# Pure-Rust WinUI Prototype

`keymap-overlay-winui` is an experimental Windows frontend that keeps the HID
listener, transition reducer, model composition, WinUI visual tree, and Win32
XAML Island host in Rust. It does not load
`keymap-overlay-windows-bridge.dll`.

The release implementation remains `windows/KeymapOverlay.Wpf`. Neither
`make build-overlay` nor `make install-overlay` selects this prototype.

## Build and run

Use the same MSYS2 UCRT64 shell as the normal Windows build:

```bash
make build-winui-overlay
target/release/keymap-overlay-winui.exe "$USERPROFILE/.config/keymap-overlay"
```

The prototype is framework-dependent and needs Windows App SDK 2.3.1 or newer
installed. Its build stages the Windows App Runtime bootstrap DLL and
`resources.pri` beside the executable.

## Why it is not the release frontend yet

- Microsoft documents C# and C++ as the supported WinUI 3 projections. The
  `windows-reactor` and `windows-reactor-setup` crates are now published on
  crates.io, but this prototype still pins the exact `windows-rs` revision it
  was developed and validated against.
- WinUI 3 does not officially support transparent top-level windows. The
  prototype therefore puts a transparent `DesktopWindowXamlSource` inside a
  layered Win32 popup. The popup provides transparency, topmost, click-through,
  and non-activation behavior while the Island keeps WinUI controls, layout,
  and typography. This composition needs visual testing around antialiased
  text and rounded edges at every DPI used.
- Text uses WinUI theme resources, but the overlay and key fills are currently
  fixed light colors. Physical testing confirms that switching Windows to dark
  mode does not update those fills. Dark-mode styling remains an open issue,
  and High Contrast still needs separate verification.
- The host stays hidden while the Island is laid out. Before every show it is
  positioned at zero opacity, waits for a DWM composition boundary, and only
  then becomes visible. Confirm that no white frame appears on either the first
  or repeated shows.
- A real desktop must confirm the non-activation contract. Type continuously
  in an editor, press and release a layer key repeatedly, and confirm the
  overlay never takes focus, especially from the second show onward.
- Hot-plug handling receives `WM_DEVICECHANGE` in the Win32 host and forwards
  it to the Rust listener. Connect a second keyboard while another remains
  attached and confirm its first layer press is recognized.

## Physical test status

- Transparent XAML Island rendering works without an opaque window background.
- DPI-aware placement and HID hot-plug recovery work on the tested desktop.
- Repeated shows remain topmost, click-through, and non-activating.
- Light-theme labels and surfaces remain readable.
- **Known issue:** dark mode does not affect the fixed overlay and key fills.
- High Contrast and additional DPI configurations still need verification.
