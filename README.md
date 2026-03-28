# ALICE-Matchmaking-SaaS

Skill-based matchmaking API — part of the ALICE Eco-System.

## Overview

ALICE-Matchmaking-SaaS delivers low-latency skill-based matchmaking for games and competitive platforms. Features Elo/Glicko-compatible rating, queue management, and match quality scoring.

## Services

- **core-engine** — Matchmaking algorithms, rating, queue (port 8124)
- **api-gateway** — JWT auth, rate limiting, reverse proxy

## Quick Start

```bash
cd services/core-engine
cargo run

curl http://localhost:8124/health
```

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | /api/v1/match/find | Find a match for player(s) |
| POST | /api/v1/match/rate | Update player rating |
| POST | /api/v1/match/queue | Join matchmaking queue |
| GET  | /api/v1/match/quality | Match quality metrics |
| GET  | /api/v1/match/stats | Service statistics |

## License

AGPL-3.0-or-later
