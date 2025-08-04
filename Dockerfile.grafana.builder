# ---- Builder stage: named 'builder' ----
FROM golang:1.23-alpine AS builder
RUN apk add --no-cache git make
RUN go install github.com/google/go-jsonnet/cmd/jsonnet@latest && \
    go install github.com/jsonnet-bundler/jsonnet-bundler/cmd/jb@latest

# ---- Final stage ----
FROM alpine:latest

# Install tools
RUN apk add --no-cache bash curl jq

# Copy jsonnet binaries from builder stage
COPY --from=builder /go/bin/jsonnet /usr/local/bin/
COPY --from=builder /go/bin/jb /usr/local/bin/

# Create separate directories
WORKDIR /app

# Copy build script to /app (not /workspace!)
COPY scripts/build.sh ./build.sh
RUN chmod +x ./build.sh

# The /workspace directory will be mounted at runtime
# The build script will work from /app but read from /workspace

CMD ["./build.sh"]
