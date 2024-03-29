FROM rust:alpine3.16 as builder
WORKDIR /src

RUN apk --no-cache add musl-dev protoc
RUN USER=root cargo init --name distribd /src
COPY Cargo.toml Cargo.lock /src/
RUN mkdir src/bin && mv src/main.rs src/bin/main.rs
RUN cargo build --target x86_64-unknown-linux-musl --release

COPY src /src/src
RUN touch src/bin/main.rs
RUN cargo build --target x86_64-unknown-linux-musl --release

FROM scratch
STOPSIGNAL SIGINT
COPY --from=builder /src/target/x86_64-unknown-linux-musl/release/distribd /distribd
CMD ["/distribd"]
