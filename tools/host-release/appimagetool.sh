#!/bin/sh
# Installs the appimagetool the Linux packages are made with, on the build image and on a native builder.
set -eu
version=1.9.1
tool=/usr/local/lib/appimagetool.AppImage
curl -sSfL "https://github.com/AppImage/appimagetool/releases/download/$version/appimagetool-$(uname -m).AppImage" -o "$tool"
chmod 755 "$tool"
