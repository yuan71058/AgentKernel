# ── Build Stage ──
FROM rust:1.87-bookworm AS builder

WORKDIR /app
COPY . .

RUN cargo build --release -p agentkernel-server --bin agentkernel

# ── Runtime Stage ──
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/agentkernel /usr/local/bin/agentkernel

# 创建数据目录
RUN mkdir -p /data/.aicore

EXPOSE 8080

# 默认环境变量（可通过 fly secrets / env 覆盖）
ENV PROTOCOL=openai
ENV MODEL=deepseek-chat
ENV BASE_URL=https://api.deepseek.com
ENV DATA_DIR=/data/.aicore

# 端口由 fly.toml 内部端口决定，这里用 8080
CMD ["agentkernel", "--addr", "0.0.0.0:8080"]
