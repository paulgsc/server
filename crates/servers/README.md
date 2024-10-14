# Visualizing SQLite Database with Metabase using Docker

This guide walks you through the process of setting up Metabase using Docker to visualize a SQLite database. We'll be using a specific SQLite database created with sqlx as an example.

## Prerequisites

- Docker installed on your system
- A SQLite database file (in this example, we're using an NFL database located at `/mnt/d/users/dev/services/server/crates/servers/nfl_server/nfl.db`)

## Step-by-Step Guide

### 1. Create a Directory for Metabase Data

First, we need to create a directory to store Metabase's own data:

```bash
mkdir -p /mnt/d/users/dev/metabase-data
```

This command creates a new directory called `metabase-data`. The `-p` flag ensures that all necessary parent directories are created if they don't already exist.

### 2. Run Metabase Docker Container

Now, we'll run the Metabase Docker container with the following command:

```bash
docker run -d -p 3000:3000 \
  -v /mnt/d/users/dev/metabase-data:/metabase-data \
  -v /mnt/d/users/dev/services/server/crates/servers/nfl_server:/data \
  -e "MB_DB_FILE=/metabase-data/metabase.db" \
  --name metabase metabase/metabase
```

Let's break down this command:

- `docker run`: This command creates and runs a new Docker container.
- `-d`: This flag runs the container in detached mode, meaning it runs in the background.
- `-p 3000:3000`: This maps port 3000 of the container to port 3000 on your host machine. This is how you'll access Metabase in your web browser.
- `-v /mnt/d/users/dev/metabase-data:/metabase-data`: This creates a volume mount. It maps the directory we created in step 1 to a directory called `/metabase-data` inside the container. This is where Metabase will store its own data.
- `-v /mnt/d/users/dev/services/server/crates/servers/nfl_server:/data`: This creates another volume mount. It maps the directory containing your SQLite database to a directory called `/data` inside the container.
- `-e "MB_DB_FILE=/metabase-data/metabase.db"`: This sets an environment variable telling Metabase where to store its own database.
- `--name metabase`: This assigns the name "metabase" to your container for easy reference.
- `metabase/metabase`: This specifies the Docker image to use.

### 3. Access Metabase

After running the Docker command, Metabase will start up. To access it:

1. Open a web browser
2. Navigate to `http://localhost:3000`

You should see the Metabase setup screen.

### 4. Configure Metabase

Follow the Metabase setup process. When you reach the database connection step:

1. Choose "SQLite" as the database type.
2. For the filename, enter `/data/nfl.db`

This path (`/data/nfl.db`) refers to the location of your SQLite database inside the Docker container, which we set up with the volume mount in step 2.

### 5. Complete Setup

Finish the Metabase setup process by following the on-screen instructions.

## Notes

- Ensure that the user running Docker has read access to your SQLite database file.
- If you're running this on a remote server, replace `localhost` with the server's IP address or domain name when accessing Metabase.
- The paths used in this guide assume you're using a bash-like shell that understands the `/mnt/d/` path. If you're on Windows using Command Prompt or PowerShell, you might need to adjust the paths accordingly.

## Troubleshooting

If you encounter any issues:

1. Check that Docker is running correctly: `docker ps` should show your Metabase container.
2. Ensure your SQLite database file has the correct permissions.
3. Check the Metabase logs: `docker logs metabase`

## Conclusion

You should now have Metabase running in a Docker container, connected to your SQLite database. You can use Metabase's interface to create visualizations and dashboards based on your NFL data.
