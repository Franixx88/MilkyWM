# MilkyWM

A Wayland compositor with a cosmic space metaphor — windows as planets orbiting a sun.

## Overview

MilkyWM is a dynamic window manager for Wayland built on Smithay, featuring an orbital window layout system. Windows are positioned and moved based on gravitational simulation principles, creating a visually distinct and interactive desktop environment.

## Features

- Full Wayland compositor implementation
- Orbital window positioning system
- XWayland support for legacy X11 applications
- DRM/KMS backend for native display support
- libinput input device handling
- OpenGL rendering via glow
- TOML-based configuration
- Logging via tracing

## Requirements

- Linux kernel with DRM/KMS support
- libseat for session management
- libinput for input handling
- GBM (Generic Buffer Management)
- udev for device enumeration

## Building

```bash
git clone https://github.com/Franixx88/MilkyWM.git
cd MilkyWM
cargo build --release