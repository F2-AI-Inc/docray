# ---- build stage ----
FROM rust:1.88-slim AS build
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates && rm -rf /var/lib/apt/lists/*
COPY . .
RUN ./scripts/fetch-pdfium.sh
RUN cargo build --release -p docray-cli -p docray-server
# The runtime image has no shell, so the data directory (and its ownership)
# is prepared here and copied across.
RUN mkdir -p /data && chown 10001:10001 /data

# ---- runtime stage ----
# Distroless: glibc, libgcc/libstdc++, CA certificates and tzdata — nothing
# else. No shell, package manager, perl, util-linux, zlib, pcre2, openssl or
# curl: those packages were the source of every CRITICAL/HIGH finding in ECR
# image scans of the previous debian-slim runtime, and the server needs none of
# them (TLS is rustls; the health check is `docray-server --healthcheck`).
FROM gcr.io/distroless/cc-debian13
COPY --from=build /src/target/release/docray /usr/local/bin/docray
COPY --from=build /src/target/release/docray-server /usr/local/bin/docray-server
COPY --from=build /src/.pdfium/lib /opt/pdfium
COPY --from=build /src/LICENSE-MIT /src/LICENSE-APACHE /src/NOTICE /src/THIRD_PARTY_NOTICES.md /usr/share/doc/docray/
COPY --from=build --chown=10001:10001 /data /data
ENV DOCRAY_PDFIUM_DIR=/opt/pdfium \
    DOCRAY_CLI_PATH=/usr/local/bin/docray \
    DOCRAY_DATA_DIR=/data \
    DOCRAY_PORT=41619
USER 10001:10001
EXPOSE 41619
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD ["/usr/local/bin/docray-server", "--healthcheck"]
ENTRYPOINT ["/usr/local/bin/docray-server"]
