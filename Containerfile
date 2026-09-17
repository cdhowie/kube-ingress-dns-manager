FROM rust:1.98.1-alpine3.24 AS builder

RUN apk add --no-cache musl-dev openssl-dev

WORKDIR /app
COPY ./ /app
RUN --mount=type=cache,target=/app/target \
    --mount=type=cache,target=/usr/local/cargo/registry/ \
    RUSTFLAGS=-Ctarget-feature=-crt-static cargo build --release && \
    cp target/release/kube-ingress-dns-manager .


FROM alpine:3.24

WORKDIR /app
RUN apk add --no-cache ca-certificates libssl3 libgcc
COPY --from=builder /app/kube-ingress-dns-manager /app/

USER nobody

CMD ["./kube-ingress-dns-manager"]
