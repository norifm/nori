FROM ubuntu:22.04
ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential curl ca-certificates python3 git pkg-config clang file squashfs-tools zstd \
    libasound2-dev libasound2-plugins libfontconfig1-dev libfreetype6-dev \
    libegl1-mesa-dev libgl1-mesa-dev libwayland-dev libxkbcommon-dev libx11-dev \
    dpkg-dev flatpak librsvg2-bin imagemagick pulseaudio dbus-user-session \
    && rm -rf /var/lib/apt/lists/*
# On arm64, x86_64's compiler and libraries: the x86_64 packages are compiled here, natively, and only
# packaged under emulation.
RUN if [ "$(dpkg --print-architecture)" = arm64 ]; then \
      sed -i 's/^deb /deb [arch=arm64] /' /etc/apt/sources.list \
      && for suite in jammy jammy-updates jammy-security; do \
           echo "deb [arch=amd64] http://archive.ubuntu.com/ubuntu $suite main universe"; done > /etc/apt/sources.list.d/amd64.list \
      && dpkg --add-architecture amd64 && apt-get update && apt-get install -y --no-install-recommends \
         gcc-x86-64-linux-gnu g++-x86-64-linux-gnu libc6-dev-amd64-cross \
         libasound2-dev:amd64 libfontconfig1-dev:amd64 libfreetype6-dev:amd64 libegl1-mesa-dev:amd64 \
         libgl1-mesa-dev:amd64 libwayland-dev:amd64 libxkbcommon-dev:amd64 libx11-dev:amd64 \
      && rm -rf /var/lib/apt/lists/*; \
    fi
ENV RUSTUP_HOME=/opt/rustup PATH=/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | CARGO_HOME=/opt/cargo sh -s -- -y --no-modify-path \
    --profile minimal --default-toolchain stable \
    && if [ "$(dpkg --print-architecture)" = arm64 ]; then /opt/cargo/bin/rustup target add x86_64-unknown-linux-gnu; fi \
    && chmod -R a+rX /opt/rustup /opt/cargo
RUN curl -sSfL "https://github.com/mozilla/sccache/releases/download/v0.18.0/sccache-v0.18.0-$(uname -m)-unknown-linux-musl.tar.gz" \
    | tar -xzf - -C /usr/local/bin --strip-components 1 --wildcards '*/sccache'
ARG NORI_UID=1000
ARG NORI_GID=1000
RUN groupadd --non-unique --gid "$NORI_GID" build && useradd --uid "$NORI_UID" --gid build --create-home build
COPY entrypoint.sh /usr/local/bin/nori-host-entrypoint
ENTRYPOINT ["/usr/local/bin/nori-host-entrypoint"]
