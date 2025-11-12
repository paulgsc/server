# Rust Dedicated Server

## Overview
yah so technically this is *a Rust server*, but actually it's a pile of crates pretending to cooperate.

no users, no traffic, just me stress-testing my own brain.

everything here is half lab, half hallucination — the kind where you start out trying to build a "dedicated backend" and end up reinventing orchestration, caching, auth, and three schedulers before lunch.

each crate kinda thinks it's the main character.

I let them cook.

> meta: the repo is a radiation leak of experiments. open source as decay.

---

## Features
* **CRUD** – standard issue. you know the drill.
* **Database Portability** – postgres, sqlite, whatever boots.
* **Role-Based Access** – fake users need fake roles.
* **Caching** – when recomputing gets personal.
* **Concurrency** – async spaghetti, but it runs (usually).

---

## Requirements
* Rust (latest stable)
* Cargo
* Some database (postgres, mysql, sqlite — or imagination)

**OR** just use Nix:
```bash
nix develop  # Everything you need, deterministically
```

See [Nix Development Environment](./nix/README.md) for details.

---

## Documentation

### Development Environment
* [**Nix Setup & Modules**](./nix/README.md) – Reproducible dev env, ML model management, no cron jobs

### System Architecture
![System Design](./docs/system_design.png)
* [System Design Documentation](./docs/system_design)
* [Mermaid Diagram Source](./docs/system-architecture.mermaid)
* [⚠️ Important Warnings](./docs/WARNING.md)

---

### Service Level Agreements
* [WebSocket Service SLA](./apps/servers/file_host/docs/sla/WebsSocket_Service_SLA.md)

---

### API Documentation
* [Google Sheets API Design](./apps/servers/file_host/docs/api/gsheet_api_design.md)

### Models Use Documentation
* [Whisper Model Optimization Guide](./docs/models/whisper-optimization.md)
