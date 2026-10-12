<p align="center">
  <img src="assets/logo.png" alt="MICRIX" width="280">
</p>

# MICRIX

Windows desktop app that mutes and unmutes the default microphone.

Download the latest x64 installer or portable zip from the [releases](https://github.com/PHUMWIWAT/micrix/releases/latest).

## What it does

- Mutes the default capture endpoint from the window, the tray, or a global shortcut.
- Records shortcuts for toggle, force mute, force unmute, hold to mute, and open the window.
- Stays in the system tray. Closing the window can hide it there.
- Can start with Windows, start hidden, and mute when the app starts.
- Shows a short mute toast. The position is a 3×3 grid. An optional WAV plays on mute and unmute.

The window stays at 1120×760. A second launch focuses the app that is already running.

## Requirements

Windows x64 with the Microsoft Edge WebView2 Runtime. Windows 11 already includes it.

The portable zip must keep `micrix.exe` and `WebView2Loader.dll` in the same folder.

## Build

```powershell
Set-Location apps/desktop
npm install
npm run tauri dev
```

A Windows GNU host needs MinGW `gcc`, `as`, and `windres` on `PATH`. An MSVC host does not.

`npm run tauri build` writes an NSIS installer for x64 under `target/release/bundle/nsis/`. A portable package is a zip of `target/release/micrix.exe` and `WebView2Loader.dll` side by side, named `MICRIX-<version>-windows-x64-portable.zip`.

Failures are appended to `%APPDATA%\com.micrix.app\micrix.log`.

## License

[Apache-2.0](LICENSE)
