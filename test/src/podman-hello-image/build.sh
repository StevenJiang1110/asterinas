#!/bin/sh

set -e

if [ ! -f "hello" ]; then
    echo "the hello binary is not found, run 'make build' at first".
    exit 1
fi

DOCKERFILE=Dockerfile.amd64
IMAGE_NAME=hello:latest

podman build -f ${DOCKERFILE} -t ${IMAGE_NAME} .