FROM rust:1.76-buster as builder

WORKDIR /app

ENV SQLX_OFFLINE true

COPY . .

RUN cargo build --release

FROM debian:buster-slim

WORKDIR /usr/local/bin

COPY --from=builder /app/target/release/rinha-rust .

RUN apt-get update && apt install -y openssl

CMD ["./rinha-rust"]