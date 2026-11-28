# Breakmine Launcher

A native GPUI desktop launcher for [Breakmine: Revived](https://github.com/Breakmine-Team/breakmine-revived).

## Run from source

From the project root:

```sh
cargo build --locked --manifest-path gpui-launcher/game-host/Cargo.toml
cargo run --locked --manifest-path gpui-launcher/Cargo.toml
```

The game opens in a separate WebView host. On Linux, that host uses GTK and WebKitGTK 4.1.

## Builds

The Build launcher workflow runs on pushes to `main` and can also be started from the Actions tab. Its artifacts contain a Windows x64 ZIP with the launcher executable and a Linux x64 AppImage. Extract the Windows ZIP before running the launcher; it needs its bundled files and Microsoft Edge WebView2 Runtime. Mark the AppImage as executable before running it. These builds are unsigned.

See the [GPUI launcher documentation](gpui-launcher/README.md) for features, data locations, and test commands. Asset credits are in [src/assets/LICENSES.txt](src/assets/LICENSES.txt).
