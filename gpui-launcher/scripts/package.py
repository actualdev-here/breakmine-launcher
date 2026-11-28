#!/usr/bin/env python3
"""Bundle prebuilt launcher and game-host binaries with their runtime assets."""
import argparse
from pathlib import Path
import plistlib
import shutil
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[2]


def package(platform, arch, launcher, host, output, webview2_loader=None):
    launcher, host, output = Path(launcher), Path(host), Path(output)
    for binary in (launcher, host):
        if not binary.is_file():
            raise ValueError(f"Binary not found: {binary}")
    if platform not in {"linux", "windows", "macos"}:
        raise ValueError("Unsupported platform")
    if arch not in {"x86_64", "aarch64"}:
        raise ValueError("Unsupported architecture")
    if platform == "windows" and (not webview2_loader or not Path(webview2_loader).is_file()):
        raise ValueError("Windows packages require the matching WebView2Loader.dll")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="breakmine-package-") as temp:
        root = Path(temp)
        if platform == "macos":
            bundle = root / "Breakmine Launcher.app" / "Contents"
            binary_dir, asset_dir = bundle / "MacOS", bundle / "Resources" / "assets"
            bundle.mkdir(parents=True)
            (bundle / "Info.plist").write_bytes(plistlib.dumps({
                "CFBundleName": "Breakmine Launcher", "CFBundleDisplayName": "Breakmine Launcher",
                "CFBundleIdentifier": "com.breakmine.launcher", "CFBundleExecutable": "breakmine-launcher",
                "CFBundlePackageType": "APPL", "CFBundleShortVersionString": "0.1.0",
                "CFBundleVersion": "1", "NSHighResolutionCapable": True,
            }))
        else:
            binary_dir = root / "Breakmine Launcher"
            asset_dir = binary_dir / "assets"
        binary_dir.mkdir(parents=True, exist_ok=True)
        suffix = ".exe" if platform == "windows" else ""
        for source, name in ((launcher, "breakmine-launcher"), (host, "breakmine-game-host")):
            target = binary_dir / (name + suffix)
            shutil.copy2(source, target)
            if platform != "windows": target.chmod(0o755)
        if platform == "windows":
            shutil.copy2(webview2_loader, binary_dir / "WebView2Loader.dll")
        shutil.copytree(ROOT / "src" / "assets", asset_dir)
        for name in ("fonts", "home-mascot.png", "microsoft-mark.svg", "mojang-mark.svg"):
            source = ROOT / "gpui-launcher" / "assets" / name
            target = asset_dir / name
            if source.is_dir(): shutil.copytree(source, target, dirs_exist_ok=True)
            else: shutil.copy2(source, target)
        notes = {
            "linux": "Run breakmine-launcher. GTK 3, WebKitGTK 4.1, Vulkan and a Secret Service keyring are required.\n",
            "windows": "Run breakmine-launcher.exe. Microsoft Edge WebView2 Runtime is required.\n",
            "macos": "This is an unsigned test build. macOS Gatekeeper may block it.\n",
        }
        (root / "README.txt").write_text(notes[platform], encoding="utf-8")
        with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
            for path in sorted(root.rglob("*")):
                if path.is_file(): archive.write(path, path.relative_to(root))
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=["linux", "windows", "macos"], required=True)
    parser.add_argument("--arch", choices=["x86_64", "aarch64"], required=True)
    parser.add_argument("--launcher", type=Path, required=True)
    parser.add_argument("--game-host", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--webview2-loader", type=Path, help="Windows WebView2Loader.dll for the target architecture")
    args = parser.parse_args()
    print(package(args.platform, args.arch, args.launcher, args.game_host, args.output, args.webview2_loader))


if __name__ == "__main__":
    main()
