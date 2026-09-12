FROM mcr.microsoft.com/devcontainers/base:ubuntu-24.04

# Rari's native image support links these even when the app uses no server images.
RUN apt-get update && apt-get install -y --no-install-recommends libfontconfig1 libfreetype6 \
    && rm -rf /var/lib/apt/lists/*
