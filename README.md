# Breakmine Launcher

A native GPUI desktop launcher for [Breakmine: Revived](https://github.com/Breakmine-Team/breakmine-revived).

## Run from source

From the project root:

```sh
cargo build --locked --manifest-path gpui-launcher/game-host/Cargo.toml
cargo run --locked --manifest-path gpui-launcher/Cargo.toml
```

The game opens in a separate WebView host. On Linux, that host uses GTK and WebKitGTK 4.1.

See the [GPUI launcher documentation](gpui-launcher/README.md) for features, data locations, and test commands. Asset credits are in [src/assets/LICENSES.txt](src/assets/LICENSES.txt).
