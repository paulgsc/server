# Redis & RedisInsight Setup

## Overview
This repository contains Docker Compose configuration for running Redis and RedisInsight (Redis' GUI management tool) in Docker containers.

## Prerequisites
- Docker and Docker Compose installed on your machine
- Basic understanding of Docker and Redis

## Quick Start

1. Clone this repository
2. Run `docker-compose up -d` to start the containers
3. Access RedisInsight at http://localhost:8001 in your browser

## Configuration Details

The `docker-compose.yml` file sets up:
- A Redis server (latest version) exposed on port 6379
- RedisInsight web interface exposed on port just check ss -tulnp prop at: http://nixos.local:5540/
- Both services connected via the 'redis-network' bridge network
- Health checks to ensure Redis is fully running before RedisInsight starts

```yaml
version: '3.7'
services:
  redis:
    image: redis:latest
    container_name: some-redis
    ports:
      - "6379:6379"
    networks:
      - redis-network
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 30s
      timeout: 10s
      retries: 5

  redisinsight:
    image: redislabs/redisinsight:latest
    container_name: redisinsight
    ports:
      - "8001:8001"
    networks:
      - redis-network
    depends_on:
      redis:
        condition: service_healthy
    restart: unless-stopped

networks:
  redis-network:
    driver: bridge
```

## Connecting to Redis via RedisInsight

After starting the containers:

1. Open your browser and go to http://localhost:8001
2. Click "Add Redis Database" or equivalent option
3. Enter the following connection details:
   - Host: `redis` (the service name in the compose file)
   - Port: `6379`
   - Name: Choose any name you prefer for this connection
4. Click "Add" or "Connect"

## Troubleshooting

### Connection Issues
If RedisInsight can't connect to Redis:
- Verify both containers are running: `docker ps`
- Check container logs: 
  ```
  docker logs some-redis
  docker logs redisinsight
  ```
- Verify network connectivity: `docker network inspect redis-network`
- Try connecting to Redis from RedisInsight using hostname `redis` instead of `localhost`

## Memba me!!
- To connect to database USE redis://redis:6379

### Port Conflicts
If you encounter port conflicts, modify the external port mappings in the docker-compose.yml file.

## Persistence
Redis data is not persisted by default in this configuration. To enable data persistence, add a volume for Redis data.

## Security Note
This configuration does not include Redis authentication. For production use, configure Redis password authentication and consider network security measures.
