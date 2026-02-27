# Dockerfile for Heiwa Gemini Strategist (The Brain)
FROM python:3.11-slim

WORKDIR /app

# Install uv for fast dependency management
RUN pip install uv

# Copy project files
COPY pyproject.toml .
COPY main.py .

# Install dependencies (assuming heiwa_sdk is accessible via git/PyPI or copied in)
# For now, install standard NATS and Gemini SDKs
RUN uv pip install --system nats-py google-generativeai

CMD ["python", "main.py"]
