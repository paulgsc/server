#!/usr/bin/env bash
set -e

echo "Building Grafana dashboard from Jsonnet..."

# Create output directory
mkdir -p /generated

# Change to the workspace directory where the source files are mounted
cd /workspace

# Ensure lib directory exists
if [ ! -d "lib" ]; then
    echo "⚠ lib/ directory not found. Make sure 'grafana-libsonnet' is mounted or installed."
    exit 1
fi

# Find all .jsonnet files (except those starting with underscore, like _helpers.jsonnet)
JSONNET_FILES=$(find . -maxdepth 1 -name '*.jsonnet' ! -name '_*.jsonnet' | sort)

if [ -z "$JSONNET_FILES" ]; then
      echo "❌ No .jsonnet files found in /workspace"
        exit 1
fi

# Build each .jsonnet file
for file in $JSONNET_FILES; do
  # Extract base filename without extension
  filename=$(basename "$file" .jsonnet)
  
  output="/generated/${filename}.json"
  
  echo "Building dashboard: $filename ..."
  jsonnet -J lib "$file" > "$output"

  # Validate and format if jq is available
  if command -v jq &> /dev/null; then
    if jq . "$output" > /dev/null; then
      echo "✓ JSON is valid: $output"
      # Format in place
      jq . "$output" > "$output.tmp" && mv "$output.tmp" "$output"
      echo "✓ JSON formatted: $output"
    else
      echo "❌ Invalid JSON generated: $output"
      exit 1
    fi
  else
    echo "⚠ jq not available, skipping validation of $output"
  fi
done

echo "✓ All Dashboards built successfully in /generated/"
echo "Build complete!"

# Debug: show what was created
ls -la /generated/

