FROM rust:1.99-bookworm AS builder

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY shadowvpn-core shadowvpn-core
COPY shadowvpn-crypto shadowvpn-crypto
COPY shadowvpn-server shadowvpn-server
COPY shadowvpn-client shadowvpn-client
COPY shadowvpn-gui shadowvpn-gui

RUN cargo build --locked --release -p shadowvpn-server

FROM debian:bookworm-slim

RUN groupadd --system shadowvpn \
    && useradd --system --gid shadowvpn --home-dir /var/lib/shadowvpn shadowvpn \
    && mkdir -p /var/lib/shadowvpn \
    && chown shadowvpn:shadowvpn /var/lib/shadowvpn

COPY --from=builder /src/target/release/shadowvpn-server /usr/local/bin/shadowvpn-server

USER shadowvpn
WORKDIR /var/lib/shadowvpn

ENV SHADOWVPN_LISTEN=0.0.0.0:8080 \
    SHADOWVPN_RELAY_LISTEN=0.0.0.0:8081 \
    SHADOWVPN_NETWORKS_FILE=/var/lib/shadowvpn/networks.json \
    SHADOWVPN_ADMIN_TOKEN_FILE=/var/lib/shadowvpn/admin-token

EXPOSE 8080/tcp
EXPOSE 8081/udp
VOLUME ["/var/lib/shadowvpn"]

ENTRYPOINT ["shadowvpn-server"]
