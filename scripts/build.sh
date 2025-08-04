#!/usr/bin/env bash
set -e

echo "Building Grafana dashboard from Jsonnet..."

# Create output directory
mkdir -p /generated

# Change to the workspace directory where the source files are mounted
cd /workspace

# Build the dashboard
jsonnet -J lib dashboard.jsonnet > /generated/dashboard.json

# Validate JSON
if command -v jq &> /dev/null; then
    echo "Validating generated JSON..."
    jq . /generated/dashboard.json > /dev/null
    echo "✓ JSON is valid"
    
    # Format the JSON nicely
    echo "Formatting JSON..."
    jq . /generated/dashboard.json > /generated/dashboard.formatted.json
    mv /generated/dashboard.formatted.json /generated/dashboard.json
    echo "✓ JSON formatted"
else
    echo "⚠ jq not available, skipping validation"
fi

echo "✓ Dashboard built successfully: /generated/dashboard.json"
echo "Build complete!"

# Debug: show what was created
ls -la /generated/

