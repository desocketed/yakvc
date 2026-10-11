# The yakvc-server image. `cargo xtask server-image` builds it from a context
# holding this file, a static (musl) yakvc-server, THIRD_PARTY_LICENSES and an
# empty state/ directory.
#
# distroless/static is an empty image plus CA certificates (for Mojang's
# session server) and a non-root user, uid 65532. Pinned by digest, since the
# tag moves: a release then builds from a known base, and `docker build
# --pull` can't swap it. To update, resolve the tag's new index digest (for
# example `crane digest gcr.io/distroless/static-debian13:nonroot`).
FROM gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3
LABEL org.opencontainers.image.source="https://github.com/desocketed/yakvc"
LABEL org.opencontainers.image.description="Yak VC rendezvous server and relay"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"

COPY yakvc-server /usr/local/bin/yakvc-server
COPY THIRD_PARTY_LICENSES /usr/share/doc/yakvc-server/THIRD_PARTY_LICENSES
# The keys and the ACME certificate cache. Created here so that Docker copies
# its owner into a fresh volume mounted on it. Owned by root because the
# systemd unit runs the server as root without CAP_DAC_OVERRIDE, so it can only
# write a directory it owns (and only read the 0600 keys it created itself).
COPY --chown=0:0 state/ /var/lib/yakvc/

# Mount config.toml here.
WORKDIR /etc/yakvc
# Relay HTTP (and ACME), relay HTTPS, QUIC address discovery, rendezvous.
EXPOSE 80/tcp 443/tcp 7842/udp 7843/udp
ENTRYPOINT ["/usr/local/bin/yakvc-server"]
CMD ["run", "--config", "/etc/yakvc/config.toml"]
