#!/usr/bin/env python3
"""Install build dependencies for autokit. Supports Debian/Ubuntu and Arch-based distros."""

import subprocess
import sys
import shutil
import os

APT_PACKAGES = [
    "build-essential", "pkg-config", "cmake",
    "libx11-dev", "libx11-xcb-dev", "libxcb1-dev",
    "libxcb-icccm4-dev", "libxcb-keysyms1-dev",
    "libxcursor-dev", "libxkbcommon-dev", "libgl-dev",
    "libasound2-dev", "libjack-dev",
]

PACMAN_PACKAGES = [
    "base-devel", "pkg-config", "cmake",
    "libx11", "libxcb", "xcb-util", "xcb-util-wm", "xcb-util-keysyms",
    "libxcursor", "libxkbcommon", "mesa", "alsa-lib", "jack2",
]


def run(cmd, **kwargs):
    print(f">>> {' '.join(cmd)}")
    subprocess.run(cmd, check=True, **kwargs)


def detect_distro():
    try:
        with open("/etc/os-release") as f:
            text = f.read().lower()
    except FileNotFoundError:
        return None
    if any(d in text for d in ("debian", "ubuntu", "pop", "mint", "elementary")):
        return "debian"
    if any(d in text for d in ("arch", "manjaro", "endeavouros", "garuda")):
        return "arch"
    return None


def install_system_deps(distro):
    if distro == "debian":
        run(["sudo", "apt-get", "update"])
        run(["sudo", "apt-get", "install", "-y"] + APT_PACKAGES)
    elif distro == "arch":
        run(["sudo", "pacman", "-S", "--needed", "--noconfirm"] + PACMAN_PACKAGES)
    else:
        print("Unknown distro. Install dependencies manually (see README.md).")
        sys.exit(1)


def install_rust():
    if shutil.which("rustup") or shutil.which("cargo"):
        print("\n-- Rust already installed, skipping --")
        return
    print("\n-- Installing Rust via rustup --")
    subprocess.run(
        "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
        shell=True, check=True,
    )
    print("\nRust installed. Run:  source ~/.cargo/env")


def main():
    distro = detect_distro()
    print(f"Detected distro family: {distro or 'unknown'}")

    print("\n-- Installing system packages --")
    install_system_deps(distro)

    install_rust()

    print("\nAll dependencies installed. Build with:")
    print("  cargo build --release")


if __name__ == "__main__":
    main()
