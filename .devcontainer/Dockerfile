# =============================================================================
# CapRust devcontainer
# - Rust toolchain (from MS image)
# - FFmpeg 7 static build (CLI only, no libs to link)
# - System deps for eframe/egui, cpal, bindgen
# - cargo-nextest for fast test runs
# =============================================================================

FROM mcr.microsoft.com/devcontainers/rust:1-1-bookworm

# -----------------------------------------------------------------------------
# 1) System dependencies
# -----------------------------------------------------------------------------
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    clang \
    libclang-dev \
    cmake \
    build-essential \
    xz-utils \
    curl \
    ca-certificates \
    # eframe / egui (X11 + GTK)
    libxcb-shape0-dev \
    libxcb-xfixes0-dev \
    libxkbcommon-dev \
    libgtk-3-dev \
    libxrandr-dev \
    libxi-dev \
    libxcursor-dev \
    # cpal (ALSA)
    libasound2-dev \
    # Handy for scripting and testing
    jq \
    && rm -rf /var/lib/apt/lists/*

# -----------------------------------------------------------------------------
# 2) FFmpeg 7 — static build (CLI only)
#    We never link libav*, only invoke ffmpeg/ffprobe as subprocess,
#    so a static binary from johnvansickle is perfect: single ~80MB
#    executable, all codecs, no shared lib deps.
# -----------------------------------------------------------------------------
RUN ARCH="$(dpkg --print-architecture)" \
    && case "$ARCH" in \
         amd64) FF_ARCH=amd64 ;; \
         arm64) FF_ARCH=arm64 ;; \
         *) FF_ARCH=amd64 ;; \
       esac \
    && curl -fsSL "https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-${FF_ARCH}-static.tar.xz" \
         -o /tmp/ffmpeg.tar.xz \
    && mkdir -p /tmp/ffmpeg-extract \
    && tar -xJf /tmp/ffmpeg.tar.xz -C /tmp/ffmpeg-extract --strip-components=1 \
    && install -m 0755 /tmp/ffmpeg-extract/ffmpeg  /usr/local/bin/ffmpeg \
    && install -m 0755 /tmp/ffmpeg-extract/ffprobe /usr/local/bin/ffprobe \
    && rm -rf /tmp/ffmpeg.tar.xz /tmp/ffmpeg-extract

# Sanity: fail fast if the download didn't work
RUN ffmpeg -version | head -1 && ffprobe -version | head -1

# -----------------------------------------------------------------------------
# 3) cargo-nextest (faster parallel test runner)
#    Nextest's install endpoint uses OS names, not Rust target triples:
#      linux / linux-arm / mac / mac-arm / windows
#    The payload is a .tar.gz containing the `cargo-nextest` binary.
# -----------------------------------------------------------------------------
RUN ARCH="$(uname -m)" \
    && case "$ARCH" in \
         x86_64)  NX_OS=linux ;; \
         aarch64) NX_OS=linux-arm ;; \
         *)       NX_OS=linux ;; \
       esac \
    && curl -LsSf "https://get.nexte.st/latest/${NX_OS}" \
       | tar zxf - -C /usr/local/cargo/bin \
    && chmod +x /usr/local/cargo/bin/cargo-nextest \
    && cargo nextest --version

# -----------------------------------------------------------------------------
# 4) Environment
# -----------------------------------------------------------------------------
# clang for bindgen (ffmpeg-sys-next build script needs it even if unused)
ENV LIBCLANG_PATH=/usr/lib/llvm-14/lib

# Make cargo faster in container
ENV CARGO_INCREMENTAL=1 \
    CARGO_NET_GIT_FETCH_WITH_CLI=true \
    CARGO_TERM_COLOR=always

# Use nextest by default where applicable
ENV NEXTEST_HIDE_PROGRESS_BAR=false

# -----------------------------------------------------------------------------
# 5) User shell tweaks
# -----------------------------------------------------------------------------
USER vscode
RUN echo 'export PATH="/usr/local/cargo/bin:$PATH"' >> ~/.bashrc \
    && echo '# Aliases' >> ~/.bashrc \
    && echo 'alias ct="cargo nextest run --workspace"' >> ~/.bashrc \
    && echo 'alias cta="cargo nextest run --workspace --all-features"' >> ~/.bashrc \
    && echo 'alias cc="cargo check --workspace"' >> ~/.bashrc \
    && echo 'alias ccb="cargo build --release -p caprust-app --no-default-features"' >> ~/.bashrc
USER root
