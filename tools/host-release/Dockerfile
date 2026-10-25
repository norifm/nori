FROM ubuntu:22.04
ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential curl ca-certificates python3 git pkg-config clang file squashfs-tools zstd \
    libasound2-dev libasound2-plugins libfontconfig1-dev libfreetype6-dev \
    libegl1-mesa-dev libgl1-mesa-dev libwayland-dev libxkbcommon-dev libx11-dev \
    libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev libgtk-3-dev \
    dpkg-dev flatpak librsvg2-bin imagemagick dbus-user-session \
    && rm -rf /var/lib/apt/lists/*
# The other architecture's compiler and libraries (x86_64's on arm64, arm64's on x86_64): both Linux
# packages are compiled here, natively, and only the other one's packaged under emulation.
RUN here="$(dpkg --print-architecture)" \
    && case "$here" in \
         arm64) other=amd64 gnu=x86_64-linux-gnu gcc=x86-64-linux-gnu mirror=http://archive.ubuntu.com/ubuntu ;; \
         amd64) other=arm64 gnu=aarch64-linux-gnu gcc=aarch64-linux-gnu mirror=http://ports.ubuntu.com/ubuntu-ports ;; \
       esac \
    && sed -i "s/^deb /deb [arch=$here] /" /etc/apt/sources.list \
    && for suite in jammy jammy-updates jammy-security; do \
         echo "deb [arch=$other] $mirror $suite main universe"; done > /etc/apt/sources.list.d/$other.list \
    && dpkg --add-architecture $other && apt-get update && apt-get install -y --no-install-recommends \
       gcc-$gcc g++-$gcc libc6-dev-$other-cross \
       libasound2-dev:$other libfontconfig1-dev:$other libfreetype6-dev:$other libegl1-mesa-dev:$other \
       libgl1-mesa-dev:$other libwayland-dev:$other libxkbcommon-dev:$other libx11-dev:$other \
       libgtk-3-dev:$other libjavascriptcoregtk-4.1-dev:$other libwebkit2gtk-4.1-0:$other libsoup-3.0-0:$other \
       libpsl5:$other libsqlite3-0:$other libbrotli1:$other libnghttp2-14:$other \
    # WebKitGTK's and libsoup's -dev packages cannot be installed for both architectures on 22.04 (installing
    # one replaces the other): the other's link files (.pc, .so links) are unpacked beside this one's; the
    # headers are the same for both.
    && mkdir /tmp/other-dev && cd /tmp/other-dev \
    && apt-get download libwebkit2gtk-4.1-dev:$other libsoup-3.0-dev:$other libnghttp2-dev:$other \
       libpolkit-gobject-1-dev:$other libsysprof-4-dev:$other libpsl-dev:$other libsqlite3-dev:$other libbrotli-dev:$other \
    && for deb in *.deb; do dpkg-deb -x "$deb" root; done \
    && cp -a root/usr/lib/$gnu/. /usr/lib/$gnu/ \
    && cd / && rm -rf /tmp/other-dev /var/lib/apt/lists/*
ENV RUSTUP_HOME=/opt/rustup PATH=/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | CARGO_HOME=/opt/cargo sh -s -- -y --no-modify-path \
    --profile minimal --default-toolchain stable \
    && /opt/cargo/bin/rustup target add "$([ "$(dpkg --print-architecture)" = arm64 ] && echo x86_64 || echo aarch64)-unknown-linux-gnu" \
    && chmod -R a+rX /opt/rustup /opt/cargo
RUN curl -sSfL "https://github.com/mozilla/sccache/releases/download/v0.18.0/sccache-v0.18.0-$(uname -m)-unknown-linux-musl.tar.gz" \
    | tar -xzf - -C /usr/local/bin --strip-components 1 --wildcards '*/sccache'
ARG NORI_UID=1000
ARG NORI_GID=1000
RUN groupadd --non-unique --gid "$NORI_GID" build && useradd --uid "$NORI_UID" --gid build --create-home build
COPY entrypoint.sh /usr/local/bin/nori-host-entrypoint
ENTRYPOINT ["/usr/local/bin/nori-host-entrypoint"]
