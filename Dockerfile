# ---- build stage ----
FROM rust:1.88-slim AS build
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates && rm -rf /var/lib/apt/lists/*
COPY . .
RUN ./scripts/fetch-pdfium.sh
RUN cargo build --release -p docray-cli -p docray-server

# ---- runtime stage ----
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 10001 docray
COPY --from=build /src/target/release/dps /usr/local/bin/docray
COPY --from=build /src/target/release/docray-server /usr/local/bin/docray-server
COPY --from=build /src/.pdfium/lib /opt/pdfium
ENV DOCRAY_PDFIUM_DIR=/opt/pdfium \
    DOCRAY_CLI_PATH=/usr/local/bin/docray \
    DOCRAY_DATA_DIR=/data \
    DOCRAY_PORT=41619
RUN mkdir -p /data && chown docray /data
USER docray
EXPOSE 41619
ENTRYPOINT ["/usr/local/bin/docray-server"]
