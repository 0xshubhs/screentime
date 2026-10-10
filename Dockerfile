# Read-only dashboard + API over the tracker's SQLite database. Tracking itself needs the
# desktop session (D-Bus, /proc, network counters), so it runs natively as a systemd user
# service; see scripts/install.sh.

FROM node:24-alpine AS ui
WORKDIR /src/ui
COPY ui/package.json ui/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY ui/ ./
RUN npm run build

FROM rust:1.96-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY data data
COPY --from=ui /src/ui/dist ui/dist
RUN cargo build --release --locked -p screentimed

FROM debian:trixie-slim
COPY --from=build /src/target/release/screentimed /usr/local/bin/screentimed
ENV HOME=/home/screentime \
    SCREENTIME_LISTEN=0.0.0.0:47821 \
    SCREENTIME_DB=/data/screentime.db
EXPOSE 47821
CMD ["screentimed", "--serve-only"]
