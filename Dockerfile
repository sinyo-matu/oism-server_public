FROM clux/muslrust AS builder

# setup sccache
# RUN apt-get update && apt-get install -y wget
# RUN wget https://github.com/mozilla/sccache/releases/download/v0.3.3/sccache-v0.3.3-x86_64-unknown-linux-musl.tar.gz \
#     && tar xzf sccache-v0.3.3-x86_64-unknown-linux-musl.tar.gz \
#     && mv sccache-v0.3.3-x86_64-unknown-linux-musl/sccache /usr/local/bin/sccache \
#     && chmod +x /usr/local/bin/sccache
# ARG AWS_ACCESS_KEY_ID
# ARG AWS_SECRET_ACCESS_KEY
# ENV RUSTC_WRAPPER=/usr/local/bin/sccache
# ENV AWS_ACCESS_KEY_ID=${AWS_ACCESS_KEY_ID}
# ENV AWS_SECRET_ACCESS_KEY=${AWS_SECRET_ACCESS_KEY}
# ENV AWS_DEFAULT_REGION=ap-northeast-1
# ENV SCCACHE_BUCKET=sccache-s3

RUN USER=root cargo new --bin oism-server 
WORKDIR /oism-server
ADD . ./
RUN cargo build --release --target x86_64-unknown-linux-musl
# RUN /usr/local/bin/sccache --show-stats

FROM alpine
ARG APP=/usr/src/app
RUN apk --no-cache add ca-certificates pkgconfig openssl-dev tzdata
RUN export SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt&&export SSL_CERT_DIR=/etc/ssl/certs
ENV TZ=Asiz/Tokyo
RUN mkdir -p ${APP}
COPY --from=builder /oism-server/target/x86_64-unknown-linux-musl/release/oism-server ${APP}/oism-server
WORKDIR ${APP}
RUN mkdir -p ./data/log&&mkdir ./configuraion
CMD ["./oism-server"]