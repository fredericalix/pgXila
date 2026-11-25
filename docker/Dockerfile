# syntax=docker/dockerfile:1
#
# pgXila server image: PostgreSQL with the pgxila extension and pgvector.
#
# Stages:
#   toolchain  PostgreSQL + Rust + cargo-pgrx, shared by every build stage
#   pgvector   pgvector built from source (no prebuilt package is assumed)
#   extension  the pgxila extension, packaged with cargo-pgrx
#   test       runs the pgrx test suite of the extension
#   runtime    the image that actually runs the server

ARG PG_MAJOR=19

FROM postgres:${PG_MAJOR}-trixie AS toolchain
ARG PG_MAJOR
ARG PGRX_VERSION=0.20.1
ARG RUST_TOOLCHAIN=stable

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential ca-certificates clang curl git libclang-dev libkrb5-dev \
      libssl-dev pkg-config postgresql-server-dev-${PG_MAJOR} sudo \
 && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo \
    PGRX_HOME=/opt/pgrx \
    PATH=/opt/cargo/bin:$PATH \
    PG_MAJOR=${PG_MAJOR} \
    PG_CONFIG=/usr/lib/postgresql/${PG_MAJOR}/bin/pg_config

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain ${RUST_TOOLCHAIN}

RUN cargo install cargo-pgrx --version ${PGRX_VERSION} --locked \
 && rm -rf /opt/cargo/registry

RUN mkdir -p ${PGRX_HOME} && cargo pgrx init --pg${PG_MAJOR}=${PG_CONFIG}


FROM toolchain AS pgvector
ARG PGVECTOR_VERSION=v0.8.7

# OPTFLAGS="" disables -march=native so the image runs on any CPU of the
# target architecture.
RUN git clone --depth 1 --branch ${PGVECTOR_VERSION} \
      https://github.com/pgvector/pgvector.git /src/pgvector \
 && make -C /src/pgvector OPTFLAGS="" PG_CONFIG=${PG_CONFIG} \
 && make -C /src/pgvector install DESTDIR=/out PG_CONFIG=${PG_CONFIG}


FROM toolchain AS extension
WORKDIR /src/extension
COPY extension/ ./
RUN --mount=type=cache,target=/opt/cargo/registry \
    --mount=type=cache,target=/src/extension/target \
    cargo pgrx package --pg-config ${PG_CONFIG} --out-dir /out


FROM toolchain AS test
WORKDIR /src/extension
COPY extension/ ./
# The test cluster cannot run as root, so pgrx starts it as the postgres user.
CMD ["sh", "-c", "cargo pgrx test pg${PG_MAJOR} --runas postgres --pgdata /tmp/pgxila-test-pgdata"]


FROM postgres:${PG_MAJOR}-trixie AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends openssl \
 && rm -rf /var/lib/apt/lists/*

COPY --from=pgvector /out/ /
COPY --from=extension /out/ /

COPY docker/postgresql.conf docker/pg_hba.conf /etc/pgxila/
COPY docker/initdb/ /docker-entrypoint-initdb.d/
COPY --chmod=755 docker/entrypoint.sh /usr/local/bin/pgxila-entrypoint.sh

ENTRYPOINT ["pgxila-entrypoint.sh"]
CMD ["postgres", "-c", "config_file=/etc/pgxila/postgresql.conf"]
