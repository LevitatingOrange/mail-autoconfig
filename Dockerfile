ARG BASE_IMAGE=clux/muslrust:stable
FROM ${BASE_IMAGE} AS builder

WORKDIR /etc/src/
ADD . ./

RUN cargo build --target x86_64-unknown-linux-musl --release

FROM alpine:latest
RUN apk --no-cache add ca-certificates
RUN apk add --no-cache tini
ENTRYPOINT ["/sbin/tini", "--"]
WORKDIR /srv

RUN echo  $'#!/bin/sh\nkill -SIGUSR1 1' > /usr/local/bin/reload-state
RUN chmod +x /usr/local/bin/reload-state
COPY --from=builder /etc/src/target/x86_64-unknown-linux-musl/release/mail-autoconfig /usr/local/bin/mail-autoconfig
COPY --from=builder /etc/src/default_config.toml /srv/config.toml

CMD ["mail-autoconfig", "run"]


