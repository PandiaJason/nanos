#!/usr/bin/env bash
set -euo pipefail

# Find the directory where this script is located
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

CONTAINER_NAME="ollama-docker-test"
PORT=11435
MODEL="qwen2.5-coder:0.5b"

echo "Checking if Docker daemon is running..."
if ! docker info >/dev/null 2>&1; then
    echo "Error: Docker daemon is not running. Please start Docker and try again."
    exit 1
fi

# Cleanup function to run on exit or interrupt
cleanup() {
    echo "Cleaning up Docker container..."
    if docker ps -a --format '{{.Names}}' | grep -Eq "^${CONTAINER_NAME}$"; then
        docker stop "${CONTAINER_NAME}" >/dev/null 2>&1 || true
        docker rm "${CONTAINER_NAME}" >/dev/null 2>&1 || true
        echo "Docker container '${CONTAINER_NAME}' stopped and removed."
    fi
}
trap cleanup EXIT

# If container already exists, clean it up first
if docker ps -a --format '{{.Names}}' | grep -Eq "^${CONTAINER_NAME}$"; then
    echo "Found existing container '${CONTAINER_NAME}'. Removing it..."
    docker stop "${CONTAINER_NAME}" >/dev/null 2>&1 || true
    docker rm "${CONTAINER_NAME}" >/dev/null 2>&1 || true
fi

echo "Starting Docker Ollama container on port ${PORT}..."
docker run -d \
    -p "${PORT}:11434" \
    --name "${CONTAINER_NAME}" \
    ollama/ollama:latest

echo "Waiting for Ollama inside Docker to be ready..."
RETRIES=30
until curl -s "http://localhost:${PORT}/" >/dev/null || [ $RETRIES -eq 0 ]; do
    sleep 1
    RETRIES=$((RETRIES-1))
done

if [ $RETRIES -eq 0 ]; then
    echo "Error: Ollama container failed to start in time."
    exit 1
fi
echo "Ollama is ready."

echo "Pulling model '${MODEL}' inside the Docker container..."
docker exec "${CONTAINER_NAME}" ollama pull "${MODEL}"

echo "Running benchmark script..."
# Ensure we run docker_vs_host.py from the benchmarks directory
python3 "${SCRIPT_DIR}/docker_vs_host.py"

echo "Benchmark finished successfully."
