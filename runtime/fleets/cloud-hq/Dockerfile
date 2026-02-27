# fleets/cloud-hq/Dockerfile
# Compatibility mirror for Railway service configs still pinned to fleets/cloud-hq.
# Keep aligned with fleets/hub/Dockerfile until the Railway service build path is normalized.
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

# Install Tailscale Static Binaries (Secure Mesh Networking)
ENV TS_VERSION=1.70.0
ENV TS_ARCH=amd64
RUN curl -fsSL "https://pkgs.tailscale.com/stable/tailscale_${TS_VERSION}_${TS_ARCH}.tgz" -o tailscale.tgz && \
    tar xzf tailscale.tgz --strip-components=1 -C /usr/local/bin tailscale_${TS_VERSION}_${TS_ARCH}/tailscaled tailscale_${TS_VERSION}_${TS_ARCH}/tailscale && \
    rm tailscale.tgz

# Ollama is optional in Cloud HQ for Wave 2A. Keep the image lean and avoid a
# multi-GB build step unless explicitly reintroduced for a dedicated lane.

# Copy the entrypoint script
COPY fleets/hub/start.sh /app/start.sh
RUN chmod +x /app/start.sh

# Copy shared libs
COPY libs/ libs/

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
