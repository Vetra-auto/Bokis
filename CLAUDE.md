# CLAUDE.md — Bokföringssystem

Svenskt bokföringssystem byggt som local-first desktop-app.

## Tech stack
- **Rust** (workspace med 5 crates)
- **Tauri 2** (desktop-shell)
- **Leptos** (WASM-frontend)
- **SQLite** (WAL mode, lokal databas)

## Arbetsflöde

1. Bygg: `cargo build`
2. Testa: `cargo test`
3. Kör desktop-app: `cargo tauri dev` (i crates/bokforing-tauri/)

## Projektstruktur

```
crates/
  bokforing-core/    — Ren domänlogik (types, ledger, correction, period, hash_chain)
  bokforing-db/      — SQLite-schema, migrationer, queries
  bokforing-export/  — SIE4, CSV, JSON-export
  bokforing-ui/      — Leptos WASM-komponenter
  bokforing-tauri/   — Tauri 2 desktop-shell, commands
```

## Nyckelprinciper

- **Append-only**: Bokförda poster (status='posted') får aldrig ändras direkt
- **Rättelser**: Alltid via reverseringspost + ny post, med CorrectionLink
- **Hashkedja**: Audit events har SHA-256 kedja (varje event hashar föregående)
- **Pengar i öre**: Alla belopp lagras som i64 i öre (10050 = 100,50 kr)
- **Periodlåsning**: open → under_review → closed → locked (permanent)
- **7 års retention**: Ingen data får raderas inom retentionstid

## Databas

- SQLite med WAL mode, foreign_keys=ON
- Migrationer körs vid appstart via sqlx::migrate!()
- UUID som TEXT, tidsstämplar som ISO 8601 TEXT
- Migrationsnamnkonvention: `YYYYMMDD######_beskrivning.sql`
