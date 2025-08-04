#!/usr/bin/env bash
set -euo pipefail

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

log() {
    echo -e "${GREEN}[$(date +'%H:%M:%S')] $1${NC}"
}

warn() {
    echo -e "${YELLOW}[$(date +'%H:%M:%S')] $1${NC}"
}

error() {
    echo -e "${RED}[$(date +'%H:%M:%S')] $1${NC}"
}

# Set build metadata
export BUILD_VERSION=$(git describe --tags --always 2>/dev/null || echo "dev-$(date +%s)")
export BUILD_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
export VCS_REF=$(git rev-parse HEAD 2>/dev/null || echo "unknown")
export RUST_LOG=${RUST_LOG:-info}

log "Building file-host server..."
log "Version: $BUILD_VERSION"
log "Build Date: $BUILD_DATE"
log "VCS Ref: $VCS_REF"

# Create data directory if it doesn't exist
mkdir -p data

# Build and start all services
log "Starting services with docker-compose..."
docker-compose up -d --build

# Wait for services to be healthy
log "Waiting for services to be ready..."
sleep 10

# Check service health
log "Checking service health..."
docker-compose ps

# Show URLs
echo ""
log "🚀 Services are running!"
log "📊 File Host Server: http://localhost:3000"
log "📈 Metrics: http://localhost:3000/metrics" 
log "🔍 Prometheus: http://localhost:9090"
log "📊 Grafana: http://localhost:3001 (admin/admin123)"
log "🗄️  Redis Insight: http://localhost:5540"
log "🔴 Redis: localhost:6379"

echo ""
log "To view logs: docker-compose logs -f file-host"
log "To stop: docker-compose down"
log "To rebuild: docker-compose up -d --build file-host"

