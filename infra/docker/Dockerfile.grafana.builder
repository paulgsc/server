# Grafana's dashboards, built from infra/grafana/dashboards/*.jsonnet into
# the JSON Grafana loads (infra/grafana/README.md; scripts/build.sh). The
# image holds only the tools and the script; the sources are mounted at
# /workspace when it runs, so a dashboard change needs no new image.
#
# Published by server's CI (.github/scripts/detect, RELEASE R6 #795, call
# C349) so home runs it by digest from its release like every other image,
# never a local build. Both bases are pinned by digest and both tools by
# version, so the same source gives the same image.

# ---- Builder stage ----
FROM golang:1.27-alpine@sha256:eb61c3873c97416fff0116553fff65d67a3549f72e11d9758849a689cdd8b523 AS builder
RUN go install github.com/google/go-jsonnet/cmd/jsonnet@v0.22.0 && \
    go install github.com/jsonnet-bundler/jsonnet-bundler/cmd/jb@v0.6.0

# ---- Final stage ----
FROM alpine:3.24@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf

RUN apk add --no-cache bash curl jq

COPY --from=builder /go/bin/jsonnet /usr/local/bin/
COPY --from=builder /go/bin/jb /usr/local/bin/

# The script runs from /app; it reads the sources mounted at /workspace and
# writes to /generated.
WORKDIR /app
COPY scripts/build.sh ./build.sh
RUN chmod +x ./build.sh

CMD ["./build.sh"]

# What this image provides and requires of the others in home's release
# (RELEASE R3, #790; docs/release.md, "Constraints"). No edge is written
# down for the dashboard builder, so it states nothing either way.
LABEL io.paulgsc.provides="" \
      io.paulgsc.requires=""
