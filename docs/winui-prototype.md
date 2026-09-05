# Pure-Rust WinUI Prototype

`keymap-overlay-winui` is an experimental Windows frontend. It keeps the HID
listener, transition reducer, model composition, WinUI visual tree, and Win32
window integration in Rust through `windows-reactor`.

The production implementation is `overlay/platforms/windows/win32`, which uses
the stable Win32 APIs directly through `windows-rs`. The prototype is not
selected by normal builds, installation, CI release acceptance, or release
packaging.

## Build and run

From an architecture-matching Visual Studio developer command prompt:

```powershell
cargo build --release --package keymap-overlay-winui
target\release\keymap-overlay-winui.exe
```

The prototype is framework-dependent and requires Windows App Runtime 2.4.0 or
newer. Its build stages the Windows App Runtime bootstrap DLL and `resources.pri`
beside the executable.

## Why it remains experimental

- `windows-reactor` is experimental, and Microsoft officially supports C# and
  C++ rather than Rust for WinUI 3 projections.
- WinUI 3 does not officially support transparent top-level windows. Reactor
  owns the WinUI window, so the prototype subclasses its HWND into a layered
  popup for transparency, topmost placement, click-through, and non-activation.
- The host stays hidden while the WinUI tree is laid out. Before every show it
  is positioned at zero opacity, waits for a DWM composition boundary, and only
  then becomes visible. A real desktop must verify that no white frame appears.
- Text uses WinUI theme resources, but the overlay and key fills are fixed light
  colors. Dark mode and High Contrast require separate validation.
- A real desktop must verify focus safety, repeated show/hide cycles, DPI-aware
  placement, click-through, topmost behavior, and Raw HID hot-plug handling.

The production backend must stay independent of these risks. Changes here do
not alter the normal Windows release path.
