# The yakvc-server image. `cargo xtask server-image` builds it from a context
# holding this file, a static (musl) yakvc-server, THIRD_PARTY_LICENSES and an
# empty state/ directory.
#
# distroless/static is an empty image plus CA certificates (for Mojang's
# session server) and a non-root user, uid 65532.
FROM gcr.io/distroless/static-debian13:nonroot
LABEL org.opencontainers.image.source="https://github.com/desocketed/yakvc"
LABEL org.opencontainers.image.description="Yak VC rendezvous server and relay"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"

COPY yakvc-server /usr/local/bin/yakvc-server
COPY THIRD_PARTY_LICENSES /usr/share/doc/yakvc-server/THIRD_PARTY_LICENSES
# The ACME certificate cache. Created here so that a fresh volume mounted on it
# is writable by the server's user.
COPY --chown=nonroot:nonroot state/ /var/lib/yakvc/

# Mount config.toml and the key files here. The example config names the keys
# relative to this directory.
WORKDIR /etc/yakvc
# Relay HTTP (and ACME), relay HTTPS, QUIC address discovery, rendezvous.
EXPOSE 80/tcp 443/tcp 7842/udp 7843/udp
ENTRYPOINT ["/usr/local/bin/yakvc-server"]
CMD ["run", "--config", "/etc/yakvc/config.toml"]
