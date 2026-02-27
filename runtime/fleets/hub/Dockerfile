# fleets/hub/Dockerfile
# Heiwa Core Collective: Spine (Brain) + Messenger (Voice)

# -----------------------------------------------------
# STAGE 1: BUILDER
# -----------------------------------------------------
FROM python:3.11-slim AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y git build-essential && rm -rf /var/lib/apt/lists/*

COPY requirements.txt .

RUN python -m venv /opt/venv
ENV PATH="/opt/venv/bin:$PATH"
RUN pip install --no-cache-dir --upgrade pip && \
    pip install --no-cache-dir -r requirements.txt

# -----------------------------------------------------
# STAGE 2: RUNTIME
# -----------------------------------------------------
FROM python:3.11-slim
WORKDIR /app

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PATH="/opt/venv/bin:$PATH" \
    LOG_LEVEL="INFO"

# Install system dependencies (curl for Tailscale, libpq for asyncpg, zstd for Ollama tar extraction)
RUN apt-get update && apt-get install -y \
    curl \
    ca-certificates \
    libpq-dev \
    zstd \
    && rm -rf /var/lib/apt/lists/*

# Copy venv from builder
COPY --from=builder /opt/venv /opt/venv

# Install NATS server for local bridge
ENV NATS_VERSION=2.10.14
RUN curl -L https://github.com/nats-io/nats-server/releases/download/v${NATS_VERSION}/nats-server-v${NATS_VERSION}-linux-amd64.tar.gz -o nats.tar.gz && \
    tar xzf nats.tar.gz && \
    mv nats-server-v${NATS_VERSION}-linux-amd64/nats-server /usr/local/bin/ && \
    rm -rf nats-server-v${NATS_VERSION}-linux-amd64 nats.tar.gz
# Tailscale binaries are optional at build-time. Cloud HQ startup has a runtime
# fallback installer and degrades without mesh if Tailscale is unavailable.
ARG HEIWA_INSTALL_TAILSCALE_BINARIES=false
ENV TS_VERSION=1.70.0
ENV TS_ARCH=amd64
RUN if [ "$HEIWA_INSTALL_TAILSCALE_BINARIES" = "true" ]; then \
      curl --retry 3 --retry-delay 2 --connect-timeout 10 --max-time 45 -fsSL \
        "https://pkgs.tailscale.com/stable/tailscale_${TS_VERSION}_${TS_ARCH}.tgz" -o tailscale.tgz && \
      tar xzf tailscale.tgz --strip-components=1 -C /usr/local/bin \
        tailscale_${TS_VERSION}_${TS_ARCH}/tailscaled \
        tailscale_${TS_VERSION}_${TS_ARCH}/tailscale && \
      rm -f tailscale.tgz; \
    else \
      echo "[HEIWA] Skipping build-time Tailscale install (HEIWA_INSTALL_TAILSCALE_BINARIES=false)."; \
    fi

# Ollama is optional in Cloud HQ for Wave 2A. Keep the image lean and avoid a
# multi-GB build step unless explicitly reintroduced for a dedicated lane.

# Copy the entrypoint script
COPY fleets/hub/start.sh /app/start.sh
RUN chmod +x /app/start.sh

# Copy shared libs
COPY libs/ libs/
COPY schemas/ schemas/
COPY policies/ policies/

# Copy the Core fleet (preserving fleets.hub package structure)
COPY requirements.txt /app/requirements.txt
COPY fleets/hub/ /app/fleets/hub/

# Create an empty fleets/__init__.py so 'fleets' is a package
RUN touch /app/fleets/__init__.py

# Cloud Identity
RUN echo '{"uuid": "heiwa-cloud-hq", "role": "orchestrator", "capabilities": ["routing", "discord"]}' > identity.json

# Expose Railway web port and Ollama local port
EXPOSE 8080 11434

# Entrypoint
CMD ["bash", "/app/start.sh"]
