#!/bin/bash
set -euo pipefail

# Shine authorizes and elevates this entire item before executing it.
source /etc/os-release
if [ "${ID:-}" != ubuntu ]; then
    echo "Docker APT bootstrap requires Ubuntu." >&2
    exit 1
fi
codename="${UBUNTU_CODENAME:-${VERSION_CODENAME:-}}"
case "$codename" in
    jammy|noble|resolute|stonking) ;;
    *) echo "Unsupported Ubuntu release: $codename" >&2; exit 1 ;;
esac
architecture="$(dpkg --print-architecture)"
case "$architecture" in
    amd64|armhf|arm64|s390x|ppc64el) ;;
    *) echo "Unsupported Docker architecture: $architecture" >&2; exit 1 ;;
esac

# Query the whole database: missing packages are normal; database errors are not.
package_status="$(dpkg-query -W -f='${Package}\t${db:Status-Status}\n')"
conflicts=()
while IFS=$'\t' read -r package status; do
    case "$package" in
        docker.io|docker-compose|docker-compose-v2|docker-doc|docker-buildx|podman-docker|containerd|runc)
            case "$status" in
                not-installed|config-files) ;;
                *) conflicts+=("$package") ;;
            esac ;;
    esac
done <<< "$package_status"
if [ "${#conflicts[@]}" -gt 0 ]; then
    printf 'Removing conflicting Docker packages: %s\n' "${conflicts[*]}"
    apt-get remove -y "${conflicts[@]}"
fi

apt-get update --error-on=any
apt-get install -y ca-certificates curl
install -m 0755 -d /etc/apt/keyrings
key="$(mktemp)"
trap 'rm -f "$key"' EXIT
curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o "$key"
install -m 0644 "$key" /etc/apt/keyrings/docker.asc
cat <<EOF | tee /etc/apt/sources.list.d/docker.sources >/dev/null
Types: deb
URIs: https://download.docker.com/linux/ubuntu
Suites: $codename
Components: stable
Architectures: $architecture
Signed-By: /etc/apt/keyrings/docker.asc
EOF
chmod 0644 /etc/apt/sources.list.d/docker.sources
apt-get update --error-on=any
apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin

if ! systemctl is-active --quiet docker; then
    systemctl start docker
fi
systemctl is-active --quiet docker
docker run --rm hello-world
