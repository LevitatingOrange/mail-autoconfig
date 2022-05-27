ARG BUILD_IMAGE=clux/muslrust:stable

FROM ${BUILD_IMAGE} AS chef
RUN cargo install cargo-chef --locked
WORKDIR /etc/src/

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
# Speed up docker build by caching dependencies
COPY --from=planner /etc/src/recipe.json recipe.json
RUN cargo chef cook --release --target x86_64-unknown-linux-musl --recipe-path recipe.json

COPY . .
RUN cargo build --target x86_64-unknown-linux-musl --release

FROM alpine:latest
RUN apk --no-cache add ca-certificates libssl1.1 libcrypto1.1
RUN apk add --no-cache tini
ENTRYPOINT ["/sbin/tini", "--"]
WORKDIR /srv

RUN echo  $'#!/bin/sh\nkill -SIGUSR1 1' > /usr/local/bin/reload-state
RUN chmod +x /usr/local/bin/reload-state
COPY --from=builder /etc/src/target/x86_64-unknown-linux-musl/release/mail-autoconfig /usr/local/bin/mail-autoconfig
COPY --from=builder /etc/src/default_config.toml /srv/config.toml
COPY --from=builder /etc/src/templates /srv/templates

CMD ["mail-autoconfig", "run"]


