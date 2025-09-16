#!/bin/sh

set -e

IMAGE_NAME=hello:latest

podman run --log-level=debug ${IMAGE_NAME}