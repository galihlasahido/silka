#!/usr/bin/env bash
# System packages the workspace needs on Linux.
#
# Kept in one script rather than repeated in three CI jobs, because the moment
# the list is duplicated one copy starts drifting and a job fails for a reason
# that has nothing to do with the change under test.
#
# What each group is for:
#
#   x11 / wayland / xkbcommon  winit's two backends (§3.1)
#   libgl / libgbm             wgpu's Vulkan and GL loaders
#   dbus / at-spi              AccessKit's Linux adapter talks over D-Bus (§3.8)
#   gtk-3 / xdo                tray icon and global menu (INTEGRASI-NATIVE §1)
#   xdg-desktop-portal         file dialogs, which we take through the portal
#                              so the result works inside a Flatpak too
set -euo pipefail

sudo apt-get update

sudo apt-get install -y --no-install-recommends \
  pkg-config \
  libx11-dev \
  libxcursor-dev \
  libxi-dev \
  libxrandr-dev \
  libxkbcommon-dev \
  libxkbcommon-x11-dev \
  libwayland-dev \
  wayland-protocols \
  libgl1-mesa-dev \
  libgbm-dev \
  libdbus-1-dev \
  libatspi2.0-dev \
  libgtk-3-dev \
  libxdo-dev \
  xdg-desktop-portal
