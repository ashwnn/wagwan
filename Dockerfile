FROM rust:1-alpine AS builder
RUN apk add --no-cache build-base musl-dev
WORKDIR /build
COPY . .
RUN cargo build --locked --release --bin wagwan \
    && mkdir -p /data \
    && chown 10001:10001 /data

FROM scratch
COPY --from=builder --chown=10001:10001 /build/target/release/wagwan /wagwan
COPY --from=builder --chown=10001:10001 /data /data
VOLUME ["/data"]
USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["/wagwan"]
CMD ["serve"]
