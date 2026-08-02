# syntax=docker/dockerfile:1.7
# SPDX-License-Identifier: Apache-2.0

FROM ubuntu:24.04

ARG TARGETARCH
ARG RUST_TOOLCHAIN=nightly-2026-08-02

LABEL org.opencontainers.image.source="https://github.com/qobilidop/xlsynth-symex"

ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_HOME=/opt/cargo \
    RUSTUP_HOME=/opt/rustup \
    PATH=/opt/cargo/bin:${PATH}

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential ca-certificates clang cmake curl git \
        libc++-18-dev libc++abi-18-dev libssl-dev libz3-dev pkg-config \
        pre-commit protobuf-compiler python3 python3-pip unzip z3 \
    && rm -rf /var/lib/apt/lists/*

RUN case "${TARGETARCH}" in \
        amd64) rust_arch=x86_64 ;; \
        arm64) rust_arch=aarch64 ;; \
        *) echo "unsupported architecture: ${TARGETARCH}" >&2; exit 1 ;; \
    esac \
    && curl --proto '=https' --tlsv1.2 --retry 5 --fail \
        "https://static.rust-lang.org/rustup/dist/${rust_arch}-unknown-linux-gnu/rustup-init" \
        -o /tmp/rustup-init \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --no-modify-path --profile minimal \
        --default-toolchain "${RUST_TOOLCHAIN}" \
    && rustup component add clippy rustfmt --toolchain "${RUST_TOOLCHAIN}" \
    && rm /tmp/rustup-init \
    && chmod -R a+rX /opt/cargo /opt/rustup

WORKDIR /workspace/xlsynth-symex

CMD ["bash"]
