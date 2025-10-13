# Tailwind CSS Linter in Rust
some tailwind linter or some graveyard project, only the future knows.


## 🎯 Project Overview

This project serves dual purposes:
1. **Build a production-ready Tailwind linter** with advanced static analysis capabilities
2. **Learn fundamental computer science concepts** through practical implementation

## 📚 Learning Journey & Timeline

### Phase 1: Foundation (Weeks 1-2)
**Theory Focus**: Basic parsing and AST concepts

#### Reading Schedule
- **Week 1**: Crafting Interpreters Chapters 1-4 (Lexical Analysis & Parsing)
- **Week 2**: Crafting Interpreters Chapters 5-8 (AST & Tree Walking)

#### Implementation Goals
- [ ] Set up project structure
- [ ] Integrate swc parser
- [ ] Build basic AST walker
- [ ] Parse simple TypeScript/JSX files

#### CS Concepts Learned
- Lexical analysis and tokenization
- Abstract syntax trees (ASTs)
- Recursive descent parsing
- Tree traversal algorithms (DFS)

### Phase 2: Pattern Recognition (Weeks 3-4)
**Theory Focus**: Pattern matching and regular expressions

#### Reading Schedule
- **Week 3**: Language Implementation Patterns Chapters 1-5 (Basic Patterns)
- **Week 4**: Regex theory from "Mastering Regular Expressions" (Chapter 1-3)

#### Implementation Goals
- [ ] Implement class extraction from string literals
- [ ] Handle JSX className attributes
- [ ] Parse template literals with embedded classes
- [ ] Build regex patterns for Tailwind class detection

#### CS Concepts Learned
- Finite state machines
- Regular expressions and pattern matching
- String algorithms
- Visitor pattern implementation

### Phase 3: Static Analysis (Weeks 5-6)
**Theory Focus**: Program analysis fundamentals

#### Reading Schedule
- **Week 5**: Static Program Analysis Chapters 1-3 (Introduction & Dataflow)
- **Week 6**: Static Program Analysis Chapter 4 (Constraint-based Analysis)

#### Implementation Goals
- [ ] Build Tailwind class database
- [ ] Implement validation rules
- [ ] Detect conflicting classes
- [ ] Track class usage patterns

#### CS Concepts Learned
- Static program analysis
- Dataflow analysis
- Constraint satisfaction
- Set theory applications

### Phase 4: Advanced Analysis (Weeks 7-8)
**Theory Focus**: Graph theory and optimization

#### Reading Schedule
- **Week 7**: Algorithm Design Manual Chapter 5 (Graph Traversal)
- **Week 8**: Principles of Program Analysis Chapters 1-2 (Semantics)

#### Implementation Goals
- [ ] Dependency tracking between classes
- [ ] Unused class detection
- [ ] Performance optimization
- [ ] Parallel processing implementation

#### CS Concepts Learned
- Graph algorithms (DFS, BFS)
- Dependency analysis
- Optimization techniques
- Parallel algorithms

## 🏗️ Project Structure

```
tailwind-linter/
├── Cargo.toml
├── README.md
├── docs/
│   ├── theory-notes.md
│   ├── architecture.md
│   └── learning-log.md
├── src/
│   ├── main.rs              # CLI entry point
│   ├── lib.rs               # Library exports
│   ├── cli.rs               # Command line interface
│   ├── config.rs            # Configuration handling
│   ├── parser/
│   │   ├── mod.rs           # Parser module exports
│   │   ├── ast_walker.rs    # AST traversal logic (Week 1-2)
│   │   └── file_parser.rs   # File parsing wrapper
│   ├── extractor/
│   │   ├── mod.rs           # Extractor module exports
│   │   ├── class_finder.rs  # Find Tailwind classes (Week 3-4)
│   │   ├── patterns.rs      # Regex patterns for extraction
│   │   └── jsx_handler.rs   # JSX-specific handling
│   ├── validator/
│   │   ├── mod.rs           # Validator module exports
│   │   ├── tailwind_db.rs   # Tailwind class database (Week 5-6)
│   │   ├── rules.rs         # Validation rules
│   │   └── checker.rs       # Class validation logic
│   ├── analyzer/
│   │   ├── mod.rs           # Advanced analysis exports
│   │   ├── dependency.rs    # Class dependency tracking (Week 7-8)
│   │   ├── usage.rs         # Usage pattern analysis
│   │   └── optimizer.rs     # Performance optimization
│   ├── reporter/
│   │   ├── mod.rs           # Reporter module exports
│   │   ├── formatter.rs     # Output formatting
│   │   └── diagnostic.rs    # Error/warning types
│   └── utils/
│       ├── mod.rs           # Utility exports
│       ├── file_utils.rs    # File system operations
│       └── string_utils.rs  # String manipulation
├── tests/
│   ├── integration/
│   │   ├── mod.rs
│   │   ├── basic_linting.rs
│   │   ├── jsx_support.rs
│   │   └── theory_validation.rs  # Tests for CS concepts
│   └── fixtures/
│       ├── valid.tsx
│       ├── invalid.tsx
│       └── config.json
├── benches/
│   ├── parsing_benchmark.rs
│   └── analysis_benchmark.rs
└── examples/
    ├── simple_usage.rs
    └── advanced_analysis.rs
```

## 📦 Dependencies

```toml
[package]
name = "tailwind-linter"
version = "0.1.0"
edition = "2021"
authors = ["Your Name <your.email@example.com>"]
description = "A fast Tailwind CSS linter with educational CS theory implementation"
license = "MIT"

[dependencies]
# Core parsing dependencies
swc_ecma_parser = "0.141"
swc_ecma_ast = "0.110"
swc_common = "0.33"
swc_ecma_visit = "0.96"

# CLI and file handling
clap = { version = "4.4", features = ["derive"] }
walkdir = "2.4"
glob = "0.3"

# Pattern matching and analysis
regex = "1.10"
once_cell = "1.19"
petgraph = "0.6"  # Graph algorithms (Week 7-8)

# Configuration and serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
toml = "0.8"

# Error handling and reporting
anyhow = "1.0"
thiserror = "1.0"
miette = { version = "5.10", features = ["fancy"] }

# Performance and concurrency
rayon = "1.8"      # Parallel processing
dashmap = "5.5"    # Concurrent HashMap
ahash = "0.8"      # Fast hashing

# Development and testing
[dev-dependencies]
tempfile = "3.8"
pretty_assertions = "1.4"
criterion = "0.5"  # Benchmarking
proptest = "1.4"   # Property-based testing
```

## 🎓 Learning Resources

### Primary Textbooks
1. **"Crafting Interpreters" by Robert Nystrom**
   - Free online: https://craftinginterpreters.com/
   - Focus: Chapters 1-8 for AST and parsing theory
   - Timeline: Weeks 1-2

2. **"Language Implementation Patterns" by Terence Parr**
   - Focus: Visitor patterns and tree walking
   - Timeline: Weeks 3-4

3. **"Static Program Analysis" by Anders Møller & Michael Schwartzbach**
   - Free PDF: https://cs.au.dk/~amoeller/spa/
   - Focus: Dataflow and constraint analysis
   - Timeline: Weeks 5-6

4. **"The Algorithm Design Manual" by Steven Skiena**
   - Focus: Graph algorithms chapter
   - Timeline: Week 7

### Supplementary Resources
- **"Mastering Regular Expressions" by Jeffrey Friedl** (Regex theory)
- **"Principles of Program Analysis"** by Nielson & Nielson (Advanced theory)
- **Rust documentation** for language-specific implementation

## 🔧 Development Phases

### Phase 1: Basic Parser (Weeks 1-2)
```bash
# Milestone 1.1: Project Setup
cargo new tailwind-linter
cd tailwind-linter
# Add swc dependencies

# Milestone 1.2: Basic AST Walking
# Implement simple file parsing
# Create visitor pattern for AST traversal

# Milestone 1.3: File Processing
# Handle multiple file types (.js, .ts, .jsx, .tsx)
# Basic error handling
```

### Phase 2: Class Extraction (Weeks 3-4)
```bash
# Milestone 2.1: String Literal Parsing
# Extract classes from "className='...'"
# Handle template literals

# Milestone 2.2: JSX Attribute Handling
# Parse JSX className attributes
# Handle conditional expressions

# Milestone 2.3: Pattern Optimization
# Optimize regex patterns
# Performance benchmarking
```

### Phase 3: Validation Engine (Weeks 5-6)
```bash
# Milestone 3.1: Tailwind Database
# Build comprehensive class database
# Handle variants and modifiers

# Milestone 3.2: Rule Engine
# Implement validation rules
# Conflict detection algorithms

# Milestone 3.3: Advanced Analysis
# Dataflow analysis for class usage
# Constraint-based validation
```

### Phase 4: Advanced Features (Weeks 7-8)
```bash
# Milestone 4.1: Dependency Analysis
# Graph-based class relationships
# Unused class detection

# Milestone 4.2: Performance Optimization
# Parallel processing implementation
# Memory optimization

# Milestone 4.3: Production Ready
# Comprehensive error reporting
# CLI polish and documentation
```

## 🧪 Testing Strategy

### Unit Tests
- Parser functionality for each file type
- Class extraction accuracy
- Validation rule correctness
- Performance regression tests

### Integration Tests
- End-to-end linting workflows
- Configuration file handling
- Error reporting accuracy

### Property-Based Tests
- Random code generation for parser robustness
- Fuzz testing for edge cases
- Performance characteristics validation

### Theory Validation Tests
- Implement textbook algorithms and verify correctness
- Compare with reference implementations
- Validate CS concepts through practical tests

## 🎯 Learning Objectives

By the end of this project, you will have practical experience with:

### Computer Science Theory
- **Parsing Theory**: Lexical analysis, syntax analysis, AST construction
- **Pattern Matching**: Regular expressions, finite state machines
- **Static Analysis**: Dataflow analysis, constraint satisfaction
- **Graph Algorithms**: Dependency analysis, traversal algorithms
- **Optimization**: Performance analysis, parallel algorithms

### Software Engineering
- **Rust Programming**: Advanced language features, memory management
- **System Design**: Modular architecture, separation of concerns
- **Testing**: Unit, integration, and property-based testing
- **Performance**: Benchmarking, profiling, optimization techniques

### Practical Skills
- **Tool Development**: Building developer tools and CLIs
- **Parser Integration**: Working with existing parsing libraries
- **Error Handling**: Robust error reporting and user experience
- **Documentation**: Technical writing and project documentation

## 🚀 Getting Started

1. **Set up the development environment**:
   ```bash
   git clone <your-repo>
   cd tailwind-linter
   cargo build
   cargo test
   ```

2. **Start with Phase 1 reading**:
   - Begin Crafting Interpreters Chapter 1
   - Set up basic project structure
   - Implement your first AST walker

3. **Document your learning**:
   - Keep notes in `docs/learning-log.md`
   - Implement theory concepts in `tests/theory_validation.rs`
   - Track progress in `docs/progress.md`

## 📈 Success Metrics

### Technical Milestones
- [ ] Parse 10,000+ lines of TypeScript/JSX without errors
- [ ] Detect 95%+ of valid Tailwind classes correctly
- [ ] Process files in under 10ms each
- [ ] Zero false positives on validation suite

### Learning Milestones
- [ ] Implement all major algorithms from textbooks
- [ ] Explain each CS concept in your own words
- [ ] Create visual demonstrations of algorithms
- [ ] Teach concepts to others (blog posts, talks)



---

*"The best way to learn computer science theory is to build something real."*
