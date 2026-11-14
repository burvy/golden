# AGENTS.md: read this before writing code in golden

You're an AI coding agent working for one of four teammates. Each teammate owns one part (see DESIGN.md, Parts).
Read `DESIGN.md` and `docs/contracts.md` first.

## Rules

ENSURE THE HUMAN UNDERSTANDS WHAT YOU ARE BUILDING

ALWAYS KEEP A DEVLOG, MAKE IT IN `docs/<person>/DEVLOG.md`

1. **Stay in your part's folder.** Need something from another part? Use its fixture in `fixtures/`, or ask your
   human to raise it with the owner. Don't edit their code.
2. **Contracts are shared.** Don't change a shape in `docs/contracts.md` inside a feature PR. Propose it in a
   tiny PR of its own, and update the matching fixture in the same PR.
3. **Functional style.** Pure functions on plain data in `crates/core`, with no IO, no globals, no clocks (pass the
   time in). IO lives at the edges (`vexcel`, `costar`, `server`). Small functions, clear names, no clever abstractions.
4. **Test what you build.** `core` needs unit tests (e.g. sun vs a known sunset time, horizon on a synthetic wall).
   Run `cargo test --workspace` (and `npm run build` in `web/` if you touched it) before you push.
5. **Secrets.** The Vexcel key comes from the `VEXCEL_API_KEY` env var (or a path in `VEXCEL_ENV_FILE`). Never print,
   log or commit it. Never commit secret keys
6. **No CoStar data in git.** The kit path comes from `COSTAR_KIT_DIR`. Small hand-made fixtures are fine.
7. **Vexcel calls are rate-limited and big.** Cache every response on disk (`cache/`, gitignored). Back off on 429.
   Never fetch a whole city.
8. **Git:** one branch per piece of work (`<part>/<what>`, e.g. `core/horizon`). Small commits that say why.
   Open a PR to `main`; the coordinator (Boss) reviews and merges. Never push to `main` directly.
9. **When unsure**, stop and tell your human. Don't guess at a contract or another part's behaviour.

## Layout (target)
```
crates/core     pure: geo, sun, horizon, score, search
crates/vexcel   IO: DSM + imagery fetch, cache, downsample
crates/costar   IO: CoStar kit loader + lookup
crates/server   axum on :8787, agent (Claude tool loop + mock)
web/            Vite + TS + MapLibre + deck.gl + three.js
docs/           contracts.md, pitch, devpost
fixtures/       one sample per contract shape
```

## Run
See README.md. Target: `cargo run -p server`, then open http://localhost:8787.
