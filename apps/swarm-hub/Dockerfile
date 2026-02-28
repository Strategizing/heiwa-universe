# apps/swarm-hub/Dockerfile
# Heiwa Core Collective: Cloud HQ (Brain)

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
    LOG_LEVEL="INFO" \
    PYTHONPATH="/app/packages:/app/packages/sdk:/app/apps"

# Install system dependencies
RUN apt-get update && apt-get install -y \
    curl \
    ca-certificates \
    libpq-dev \
    zstd \
    && rm -rf /var/lib/apt/lists/*

# Copy venv from builder
COPY --from=builder /opt/venv /opt/venv

# Install NATS server
ENV NATS_VERSION=2.10.14
RUN curl -L https://github.com/nats-io/nats-server/releases/download/v${NATS_VERSION}/nats-server-v${NATS_VERSION}-linux-amd64.tar.gz -o nats.tar.gz && \
    tar xzf nats.tar.gz && \
    mv nats-server-v${NATS_VERSION}-linux-amd64/nats-server /usr/local/bin/ && \
    rm -rf nats-server-v${NATS_VERSION}-linux-amd64 nats.tar.gz

# Copy Architecture
COPY packages/ /app/packages/
COPY config/ /app/config/
COPY apps/swarm-hub/ /app/apps/swarm-hub/
COPY requirements.txt /app/requirements.txt

# Copy entrypoint
COPY apps/swarm-hub/start.sh /app/start.sh
RUN chmod +x /app/start.sh

# Cloud Identity
RUN echo '{"uuid": "heiwa-cloud-hq", "role": "orchestrator", "capabilities": ["routing", "discord"]}' > identity.json

# Expose ports
EXPOSE 8080 4222

# Entrypoint
CMD ["bash", "/app/start.sh"]
