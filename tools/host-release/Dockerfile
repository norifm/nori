FROM ubuntu:22.04
ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential curl ca-certificates python3 git pkg-config clang file squashfs-tools zstd \
    libasound2-dev libasound2-plugins libfontconfig1-dev libfreetype6-dev libdbus-1-dev \
    libegl1-mesa-dev libgl1-mesa-dev libwayland-dev libxkbcommon-dev libx11-dev \
    dpkg-dev flatpak librsvg2-bin imagemagick pulseaudio dbus-user-session \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/opt/rustup PATH=/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | CARGO_HOME=/opt/cargo sh -s -- -y --no-modify-path \
    --profile minimal --default-toolchain stable \
    && chmod -R a+rX /opt/rustup /opt/cargo
RUN curl -sSfL "https://github.com/mozilla/sccache/releases/download/v0.18.0/sccache-v0.18.0-$(uname -m)-unknown-linux-musl.tar.gz" \
    | tar -xzf - -C /usr/local/bin --strip-components 1 --wildcards '*/sccache'
ARG NORI_UID=1000
ARG NORI_GID=1000
RUN groupadd --non-unique --gid "$NORI_GID" build && useradd --uid "$NORI_UID" --gid build --create-home build
COPY entrypoint.sh /usr/local/bin/nori-host-entrypoint
ENTRYPOINT ["/usr/local/bin/nori-host-entrypoint"]
