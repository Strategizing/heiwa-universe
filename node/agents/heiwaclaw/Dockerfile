# Dockerfile for HeiwaClaw Duplicatable Instances
FROM python:3.11-slim

WORKDIR /app

# Install system dependencies for psutil and network
RUN apt-get update && apt-get install -y \
    gcc \
    python3-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy requirements and install
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Copy source code
COPY main.py .
COPY ui_manager.py .
COPY swarm_bridge.py .

# Environment Defaults
ENV AGENT_NAME=HeiwaClaw
ENV NODE_ID=HeiwaClaw-Macbook
ENV NATS_URL=nats://host.docker.internal:4222
ENV OLLAMA_URL=http://host.docker.internal:11434
ENV MODEL_PRIMARY=deepseek-coder-v2:16b
ENV MODEL_SECONDARY=qwen2.5-coder:7b
ENV PERSONALITY="A professional Heiwa Swarm agent."

CMD ["python", "main.py"]
