# Lint Parser MVP - Sprint 1

A Rust-based CLI tool that parses clippy and ESLint JSON output, extracts structured lint issues, and generates AI-powered fixes using a local Ollama instance.

## Features

- **Clippy Parser**: Parses clippy JSON output into structured `ClippyIssue` data
- **ESLint Parser**: Parses ESLint JSON output into structured `ESLintIssue` data  
- **Ollama Integration**: Uses local LLM (codellama:7b) to generate code fixes
- **Docker Support**: Run Ollama via Docker container
- **CLI Interface**: Simple command-line interface for processing lint output

## Quick Start

### 1. Start Ollama

```bash
# Build and start Ollama with codellama:7b
docker-compose up -d

# Wait for the model to download (first time only)
docker-compose logs -f ollama
```

### 2. Build and Run

```bash
# Build the Rust application
cargo build --release

# Parse clippy output
cargo run clippy sample_clippy.json

# Parse ESLint output  
cargo run eslint sample_eslint.json

# Or pipe from stdin
clippy --message-format=json | cargo run clippy -
eslint --format=json | cargo run eslint -
```

## Usage Examples

### Clippy Usage
```bash
# Generate clippy JSON output
cargo clippy --message-format=json 2> clippy_output.json

# Parse and get AI fixes
cargo run clippy clippy_output.json
```

### ESLint Usage
```bash
# Generate ESLint JSON output
npx eslint --format=json . > eslint_output.json

# Parse and get AI fixes  
cargo run eslint eslint_output.json
```

## Data Structures

### ClippyIssue
```rust
struct ClippyIssue {
    file_path: String,      // Path to the file with the issue
    line: u32,              // Line number
    column: u32,            // Column number  
    rule: String,           // Clippy rule (e.g., "clippy::unused_mut")
    message: String,        // Issue description
    suggestion: Option<String>, // Clippy's suggested fix
    code_snippet: String,   // Code context around the issue
}
```

### ESLintIssue
```rust  
struct ESLintIssue {
    file_path: String,      // Path to the file with the issue
    line: u32,              // Line number
    column: u32,            // Column number
    rule_id: String,        // ESLint rule (e.g., "no-unused-vars")
    message: String,        // Issue description  
    fix: Option<ESLintFix>, // ESLint's automatic fix
    code_snippet: String,   // Code context around the issue
}
```

## Architecture

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   Clippy JSON   │───▶│  Clippy Parser  │───▶│  ClippyIssue    │
└─────────────────┘    └─────────────────┘    └─────────────────┘
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│  ESLint JSON    │───▶│  ESLint Parser  │───▶│  ESLintIssue    │
└─────────────────┘    └─────────────────┘    └─────────────────┘
                                                       │
                                                       ▼
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   Ollama API    │◀───│  Ollama Client  │◀───│   AI Fixes      │
└─────────────────┘    └─────────────────┘    └─────────────────┘
```

## Testing

### Run Unit Tests
```bash
cargo test
```

### Run Integration Tests
```bash
cargo test integration_tests
```

### Test with Sample Data
```bash
# Test clippy parsing
echo '{"message":{"message":"unused variable: `x`","code":{"code":"unused_variables"},"level":"warning","spans":[{"file_name":"src/main.rs","line_start":5,"line_end":5,"column_start":9,"column_end":10,"text":[{"text":"    let x = 5;"}]}],"children":[]}}' | cargo run clippy -

# Test ESLint parsing  
echo '[{"filePath":"test.ts","messages":[{"ruleId":"no-unused-vars","severity":2,"message":"'"'"'unused'"'"' is defined but never used.","line":3,"column":7}]}]' | cargo run eslint -
```

## Docker Commands

```bash
# Start Ollama service
docker-compose up -d

# Check Ollama logs
docker-compose logs ollama

# Stop services
docker-compose down

# Rebuild Ollama image
docker-compose up --build -d
```

## Configuration

The Ollama client defaults to:
- **Base URL**: `http://localhost:11434`  
- **Model**: `codellama:7b`
- **Docker Port**: `11434`

## Dependencies

- **serde**: JSON serialization/deserialization
- **reqwest**: HTTP client for Ollama API
- **tokio**: Async runtime
- **anyhow**: Error handling
- **thiserror**: Custom error types

## Definition of Done ✓

- [x] Clippy JSON output parser working
- [x] ESLint JSON output parser working  
- [x] Basic ollama integration (codellama:7b)
- [x] Unit tests for parsers
- [x] Ollama runs via Docker container
- [x] CLI interface for both lint types
- [x] Code context extraction
- [x] Health checks for Ollama availability

## Next Steps

Sprint 2 will add:
- File modification capabilities
- Batch processing
- Configuration management  
- Enhanced error handling
- More comprehensive test coverage
