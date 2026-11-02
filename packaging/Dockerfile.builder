FROM rust:1.98.1-alpine@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f
RUN apk add --no-cache git python3 binutils curl tar zstd
# Build tools only. They are never dependencies of the installed headless service.
ENV CARGO_BUILD_JOBS=1 HPS_FORBID_HOST_NETWORK_CHANGES=1
