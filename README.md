# MICRIX

Desktop app that mutes and unmutes the default microphone.

```powershell
Set-Location apps/desktop
npm install
npm run tauri dev
```

A Windows GNU host needs MinGW `gcc`, `as`, and `windres` on `PATH`. An MSVC host does not.

`npm run tauri build` writes an NSIS installer for x64 under `target/release/bundle/nsis/`. A portable package is a zip of `target/release/micrix.exe` named `MICRIX-<version>-windows-x64-portable.zip`. Failures are appended to `%APPDATA%\com.micrix.app\micrix.log`.
