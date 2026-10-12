# Vendored winit drag-position patch

Versus uses the pointer location that `winit` reports with a native file drop to decide whether
the item belongs to the left or right comparison pane. The upstream `winit` 0.30 source currently
vendored in this repository forwards the paths but omits that location for OLE, XDND, and AppKit
file drags.

The local patch sends a client-relative physical `WindowEvent::CursorMoved` before hover and drop
events in these files:

- `vendor/winit/src/platform_impl/windows/drop_handler.rs`
- `vendor/winit/src/platform_impl/windows/definitions.rs`
- `vendor/winit/src/platform_impl/linux/x11/event_processor.rs`
- `vendor/winit/src/platform_impl/macos/window_delegate.rs`

Windows converts OLE screen pixels with `ScreenToClient`; X11 converts packed XDND root-window
coordinates with `TranslateCoordinates`; AppKit converts the drag point through Winit's flipped
content view and then applies the backing scale factor.

The Windows patch also corrects the OLE callback ABI: `DragEnter`, `DragOver`, and `Drop`
take `POINTL` **by value**, as required by the Windows SDK. The upstream declarations used a
pointer; on x64 the mismatch was masked while ignored, but dereferencing it interpreted packed
coordinates as an address. Both the vtable declarations and callback implementations must use the
same by-value signature. The handler implements `QueryInterface` for `IUnknown`/`IDropTarget`,
negotiates only permitted copy effects, clears completed/cancelled drags, and releases every
successful `GetData` storage medium with `ReleaseStgMedium`, including hover reads and
provider-owned media. Dragging selects comparison inputs without moving or deleting sources.

Windows-only integration tests in `tests/windows_drop.rs` compile the actual patched handler
with an event-recording adapter and exercise its vtable using a hidden native window and real
`CF_HDROP` payloads. They check client coordinates, event order, Unicode/long paths, medium
release, effects, cancellation, and interface queries. `cargo test --locked --offline` runs
them on Windows CI; the adapter does not replace the OLE callback implementation. Manual
Explorer drops into the application window remain useful for end-to-end validation.

References: [DragEnter ABI](https://learn.microsoft.com/en-us/windows/win32/api/oleidl/nf-oleidl-idroptarget-dragenter),
[DragOver ABI](https://learn.microsoft.com/en-us/windows/win32/api/oleidl/nf-oleidl-idroptarget-dragover),
[Drop ABI](https://learn.microsoft.com/en-us/windows/win32/api/oleidl/nf-oleidl-idroptarget-drop), and
[storage-medium ownership](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-releasestgmedium).

The vendored Winit 0.30 Wayland backend has no data-device drag-and-drop implementation and emits
no `HoveredFile` or `DroppedFile` events. Its ordinary pointer-motion support cannot make native
file drops work, so it is intentionally unchanged. Versus chooses X11/XWayland when `DISPLAY`
is nonempty through its native event-loop builder hook. Without X11, native Wayland retains
browse selection. A stale or unavailable `DISPLAY` reports a normal window-startup error.

`scripts/vendor-dependencies.ps1` recreates `vendor/` and replaces these edited source files.
The reviewable source diff is saved in `docs/winit-drag-position.patch`. After refreshing the
same Winit version in `vendor/winit`, reapply it from the repository root with
`git apply docs/winit-drag-position.patch`. If Cargo uses a versioned directory name or Winit's
source has changed, adapt the patch paths and review its coordinate conversions before applying.
Then recalculate the matching
SHA-256 entries in `vendor/winit/.cargo-checksum.json`. The checksum uses the lowercase hexadecimal
digest of each file, for example:

```powershell
(Get-FileHash vendor/winit/src/platform_impl/windows/drop_handler.rs -Algorithm SHA256).Hash.ToLower()
```

Finish by running the offline release build documented in the project reference.
