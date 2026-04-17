# SPDX-License-Identifier: MPL-2.0

ARG BASE_VERSION
FROM asterinas/asterinas:${BASE_VERSION}

SHELL ["/bin/bash", "-c"]

ARG DEBIAN_FRONTEND=noninteractive
ARG CRICTL_VERSION
ARG KATA_INSTALL_CRICTL
ARG KATA_STATIC_TARBALL_SHA256
ARG KATA_STATIC_TARBALL_SHA256_URL
ARG KATA_STATIC_TARBALL_URL
ARG KATA_VERSION
ARG NERDCTL_VERSION

COPY tools/kata/config/smoke-test.env /tmp/kata-smoke-test.env

RUN set -euxo pipefail; \
    source /tmp/kata-smoke-test.env; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        busybox-syslogd \
        containernetworking-plugins \
        containerd \
        iptables \
        jq \
        kmod \
        python3 \
        runc \
        strace \
        wget \
        zstd; \
    wget -O /tmp/nerdctl.tgz \
        "https://github.com/containerd/nerdctl/releases/download/${NERDCTL_VERSION}/nerdctl-${NERDCTL_VERSION#v}-linux-amd64.tar.gz"; \
    tar -C /usr/local/bin -xzf /tmp/nerdctl.tgz nerdctl; \
    case "${KATA_INSTALL_CRICTL:-0}" in \
        1 | true | TRUE | yes | YES) \
            wget -O /tmp/crictl.tgz \
                "https://github.com/kubernetes-sigs/cri-tools/releases/download/${CRICTL_VERSION}/crictl-${CRICTL_VERSION}-linux-amd64.tar.gz"; \
            tar -C /usr/local/bin -xzf /tmp/crictl.tgz crictl; \
            ;; \
    esac; \
    static_tarball_name="$(basename "${KATA_STATIC_TARBALL_URL%%\?*}")"; \
    if [ -n "${KATA_STATIC_TARBALL_SHA256:-}" ]; then \
        expected_sha256="${KATA_STATIC_TARBALL_SHA256}"; \
    else \
        checksum_url="${KATA_STATIC_TARBALL_SHA256_URL:-}"; \
        if [ -z "${checksum_url}" ]; then \
            case "${KATA_STATIC_TARBALL_URL}" in \
                *.tar.zst) \
                    checksum_url="${KATA_STATIC_TARBALL_URL%.tar.zst}.SHA256SUMS" \
                    ;; \
                *) \
                    echo "Cannot derive SHA256SUMS URL from ${KATA_STATIC_TARBALL_URL}" >&2; \
                    exit 1 \
                    ;; \
            esac; \
        fi; \
        wget -O /tmp/kata-static.SHA256SUMS "${checksum_url}"; \
        expected_sha256="$( \
            awk -v asset_name="${static_tarball_name}" \
                '$2 == asset_name || $2 ~ ("/" asset_name "$") { print $1; exit }' \
                /tmp/kata-static.SHA256SUMS \
        )"; \
        test -n "${expected_sha256}"; \
    fi; \
    wget -O /tmp/kata-static.tar.zst "${KATA_STATIC_TARBALL_URL}"; \
    echo "${expected_sha256}  /tmp/kata-static.tar.zst" | sha256sum -c -; \
    rm -rf /opt/kata /tmp/kata-static-extract; \
    install -d -m 0755 /tmp/kata-static-extract; \
    tar --zstd -xf /tmp/kata-static.tar.zst -C /tmp/kata-static-extract; \
    test -d /tmp/kata-static-extract/opt/kata; \
    cp -a /tmp/kata-static-extract/opt/kata /opt/; \
    { \
        printf 'static-tarball-url %s\n' "${KATA_STATIC_TARBALL_URL}"; \
        printf 'static-tarball-sha256 %s\n' "${expected_sha256}"; \
    } > /opt/kata/.kata-install-source; \
    ln -sf /opt/kata/bin/kata-runtime /usr/local/bin/kata-runtime; \
    ln -sf /opt/kata/bin/containerd-shim-kata-v2 /usr/local/bin/containerd-shim-kata-v2; \
    nerdctl --version; \
    kata-runtime --version | grep -F "${KATA_VERSION}"; \
    apt-get clean; \
    rm -rf /var/lib/apt/lists/*; \
    rm -f /tmp/nerdctl.tgz /tmp/crictl.tgz /tmp/kata-static.tar.zst /tmp/kata-static.SHA256SUMS /tmp/kata-smoke-test.env; \
    rm -rf /tmp/kata-static-extract
