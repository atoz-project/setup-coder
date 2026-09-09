# syntax=docker/dockerfile:1

# ARC 弹性 pod 自定义 runner 镜像:官方极简镜像 + Rust 项目工具链预装。
# 目标:CI 冷 pod 零安装(现状每次 apt build-essential ~25s + rustup 工具链下载 ~15s)。
# 基底与线上 pod 一致(ubuntu noble 极简镜像,runner v2.337.0),只增不删。
FROM ghcr.io/actions/actions-runner:2.337.0

USER root

# build-essential:cc 链接器(cargo 必需,极简镜像没有——曾致 proc-macro2 构建失败)
# musl-tools:setup-coder build 矩阵 linux-x64 静态 musl 目标备用(将来迁上 ARC 零新增)
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential musl-tools \
 && rm -rf /var/lib/apt/lists/*

# rustup 装到共享位置(沿用官方 rust 镜像的 RUSTUP_HOME/CARGO_HOME 约定),
# a+w 让 runner 用户可写 cargo registry 缓存与 rustup 自更新
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

# 钉小版本:CI 的 clippy 门禁对工具链版本敏感,升级 = 改这里 + 重建镜像,可审计。
RUN curl -sSf https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init -o /tmp/rustup-init \
 && chmod +x /tmp/rustup-init \
 && /tmp/rustup-init -y --no-modify-path --profile minimal \
      --default-toolchain 1.98.1 --component clippy rustfmt \
 && rustup target add x86_64-unknown-linux-musl \
 && rm /tmp/rustup-init \
 && chmod -R a+w /usr/local/rustup /usr/local/cargo \
 && rustc --version && cargo --version && cargo clippy --version

USER runner
