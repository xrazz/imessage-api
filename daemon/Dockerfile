FROM rust:1.95-bookworm AS build

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl protobuf-compiler tar \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace
COPY daemon ./daemon
COPY vendor/rustpush ./vendor/rustpush
COPY vendor/openbubbles-build-modules ./vendor/openbubbles-build-modules

# Pull the OpenBubbles release library the build-modules project is designed to use.
RUN curl -L \
      'https://github.com/OpenBubbles/openbubbles-app/releases/download/v1.15.0%2B136/bluebubbles-linux-x86_64.tar' \
      -o /tmp/bluebubbles-linux-x86_64.tar \
    && mkdir -p /tmp/openbubbles-release \
    && tar -xf /tmp/bluebubbles-linux-x86_64.tar -C /tmp/openbubbles-release ./lib/librust_lib_bluebubbles.so \
    && cp /tmp/openbubbles-release/lib/librust_lib_bluebubbles.so /workspace/vendor/openbubbles-build-modules/openbubbles.so \
    && echo 'f47fbd299bf5c83449bf6485a2c00c0f059d0e059646e20c64111bc5fac84b2a  /workspace/vendor/openbubbles-build-modules/openbubbles.so' | sha256sum -c -

# Extract real FairPlay certs for activation and build the Linux validation helper.
RUN cd /workspace/vendor/openbubbles-build-modules \
    && cargo run --package fairplay-certs --bin fairplay-certs \
    && mkdir -p /workspace/vendor/rustpush/certs/fairplay \
    && cp target/fairplay_certs/* /workspace/vendor/rustpush/certs/fairplay/ \
    && cargo build --release --package macos-validation-data

RUN cd /workspace/daemon && cargo build --release

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /workspace/daemon/target/release/imessage-daemon /usr/local/bin/imessage-daemon
COPY --from=build /workspace/vendor/openbubbles-build-modules/target/release/macos-validation-data /usr/local/bin/macos-validation-data
COPY --from=build /workspace/vendor/openbubbles-build-modules/openbubbles.so /usr/local/lib/openbubbles.so
ENV LD_LIBRARY_PATH=/usr/local/lib
ENV MACOS_VALIDATION_DATA_BIN=/usr/local/bin/macos-validation-data
CMD ["imessage-daemon"]
