# Dockerfile for Heiwa OpenClaw (Deep Researcher)
FROM python:3.11-slim

WORKDIR /app

# Install uv for fast dependency management
RUN pip install uv

# Install Playwright system dependencies
RUN apt-get update && apt-get install -y \
    wget \
    gnupg \
    libnss3 \
    libatk-bridge2.0-0 \
    libxcomposite1 \
    libxdamage1 \
    libxrandr2 \
    libgbm1 \
    libasound2 \
    && rm -rf /var/lib/apt/lists/*

# Copy project files
COPY pyproject.toml .
COPY main.py .

# Install dependencies
RUN uv pip install --system nats-py playwright pydantic

# Install Playwright browsers
RUN uvx playwright install chromium

CMD ["python", "main.py"]
