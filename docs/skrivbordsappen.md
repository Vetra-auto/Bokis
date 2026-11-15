# Skrivbordsappen: den första versionen

Det här gäller den första versionen av Bokis, en skrivbordsapp i Tauri. Den finns kvar tills
AI-first-versionen har ersatt den.

## Funktioner

- **Verifikationer** — Skapa, bokför och sök verifikationer med automatisk verifikationsnummerräknare
- **Grundbok & Huvudbok** — Fullständig journal och kontovisa rapporter
- **Rättelseflöde** — Reversering + ersättning med spårbar koppling (BFL 5 kap. 5§)
- **Periodhantering** — Öppen → Under granskning → Stängd → Låst (permanent)
- **Kontoplan** — BAS 2024 (klass 1–9), SRU-koder
- **Motparter** — Register med organisationsnummer och kontaktinfo
- **Export** — SIE4 (CP437), CSV, JSON, PDF (grundbok/balansrapport)
- **Backup** — VACUUM INTO + SHA-256 verifiering
- **Arkivering** — 7 års retention (BFL 7 kap. 2§), arkivmanifest med hash
- **Revisionslogg** — SHA-256 hashkedja (append-only, manipulationsdetektering)
- **Behandlingshistorik** — Spårar alla åtgärder på bokföringsmaterial
- **Systemdokumentation** — Automatgenererad snapshot per BFNAR 2013:2
- **Fleranvändarstöd** — Admin, bokförare, attestant, revisor (rollbaserad åtkomstkontroll)

## Tech stack

| Komponent | Teknologi |
|-----------|-----------|
| Desktop-shell | Tauri 2 |
| Frontend | Leptos (Rust → WASM) |
| Databas | SQLite (WAL mode) |
| Lösenord | Argon2id |
| Hashkedja | SHA-256 |
| Export | SIE4 (codepage-437), CSV, JSON, genpdf |

Hela systemet är skrivet i Rust — ingen JavaScript, ingen extern server.

## Projektstruktur

```
bokforing/
├── Cargo.toml                    # Workspace
├── CLAUDE.md                     # AI-assistentinstruktioner
├── build-appimage.sh             # AppImage-byggskript
├── crates/
│   ├── bokforing-core/           # Ren domänlogik, inga I/O-beroenden
│   │   └── src/
│   │       ├── types.rs          # Money(i64 öre), AccountNumber, Role, Status-enums
│   │       ├── ledger.rs         # Bokföringsmotor, balansvalidering
│   │       ├── correction.rs     # Rättelseflöde (reversering + ersättning)
│   │       ├── period.rs         # Period-tillståndsmaskin
│   │       ├── hash_chain.rs     # SHA-256 kedjeberäkning + verifiering
│   │       ├── fiscal_year.rs    # Räkenskapsår + periodgenerering
│   │       └── validation.rs     # Domänvalideringsregler
│   ├── bokforing-db/             # SQLite-schema, migrationer, queries
│   │   ├── migrations/           # SQL-migrationer (körs vid appstart)
│   │   ├── src/queries/          # En modul per entitet
│   │   └── tests/                # Integrationstester (in-memory SQLite)
│   ├── bokforing-export/         # Exportformat
│   │   └── src/
│   │       ├── sie4.rs           # SIE4 (svensk standard, CP437)
│   │       ├── csv_export.rs     # Semikolonseparerad CSV
│   │       ├── json_export.rs    # Komplett JSON-export
│   │       └── pdf_export.rs     # PDF via genpdf (Liberation Sans)
│   ├── bokforing-ui/             # Leptos WASM-frontend
│   │   ├── src/pages/            # 14 sidor (dashboard, vouchers, journal, ...)
│   │   └── style/main.css        # All CSS
│   └── bokforing-tauri/          # Tauri desktop-shell
│       ├── tauri.conf.json
│       └── src/
│           ├── main.rs           # App setup, migrations, plugin-registrering
│           ├── state.rs          # AppState (pool, current_user, current_company)
│           ├── error.rs          # CommandError enum
│           └── commands/         # 14 command-moduler
```

## Krav

- Rust 1.80+ (stable)
- `wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`)
- Trunk (`cargo install trunk`)
- Tauri CLI (`cargo install tauri-cli`)
- System: GTK3, WebKitGTK (`sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev`)
- Fonter för PDF: Liberation Sans (`sudo apt install fonts-liberation`)

## Bygg & kör

```bash
# Utveckling
cargo tauri dev

# Tester
cargo test --workspace

# Release (.deb)
cargo tauri build --bundles deb

# Release (AppImage) — kräver librsvg2-dev
./build-appimage.sh

# Installera .deb
sudo dpkg -i target/release/bundle/deb/Bokis_0.1.0_amd64.deb
```

## Installation (nedladdning av senaste release)

Bokis distribueras som .deb-paket och AppImage för Linux x86_64. Välj det alternativ som passar din distribution.

### .deb (Ubuntu/Debian)

```bash
# Ladda ner och installera senaste releasen
curl -sLO https://github.com/Vetra-auto/Bokis/releases/latest/download/Bokis_0.1.0_amd64.deb
sudo dpkg -i Bokis_0.1.0_amd64.deb
sudo apt-get install -f  # installera eventuella beroenden
```

### AppImage (alla Linux-distributioner)

```bash
# Ladda ner senaste AppImage
curl -sLO https://github.com/Vetra-auto/Bokis/releases/latest/download/Bokis_0.1.0_amd64.AppImage
chmod +x Bokis_0.1.0_amd64.AppImage
./Bokis_0.1.0_amd64.AppImage
```

### Via GitHub CLI

```bash
# .deb
gh release download --repo Vetra-auto/Bokis --pattern '*.deb'
sudo dpkg -i Bokis_*_amd64.deb

# AppImage
gh release download --repo Vetra-auto/Bokis --pattern '*.AppImage'
chmod +x Bokis_*.AppImage
./Bokis_*.AppImage
```

### Avinstallera

```bash
# .deb
sudo apt remove bokis

# AppImage — ta bara bort filen
rm Bokis_*.AppImage
```

## Datalagring

All data lagras lokalt i användarens app-datakatalog:

```
~/.local/share/se.bokforing.bokis/
├── bokforing.db              # SQLite-databas (WAL mode)
├── attachments/              # Bilagor (per verifikation)
├── exports/                  # Exporterade filer
├── backups/                  # Databasbackuper
└── archives/                 # Årsarkiv
```

## Nyckeldesign

### Pengar
Alla belopp lagras som `INTEGER` i öre (10050 = 100,50 kr). Inga flyttal.

### Append-only
Bokförda verifikationer (`status = 'posted'`) är oföränderliga. Rättelse sker via:
1. Reverseringspost (omvända debet/kredit)
2. Ersättningspost (korrekt bokning)
3. `correction_links`-tabell kopplar tripletten

### SHA-256 hashkedja
```
event_hash = SHA-256(seq|type|entity_id|occurred_at|detail_json|previous_hash)
genesis: previous_hash = "0" × 64
```
En bruten kedja indikerar manipulation — verifiering via UI eller command.

### Periodlivscykel
```
open → under_review → closed → locked
  ↑         |
  +---------+  (reopen, admin + loggat)
```
`locked` = permanent, kan aldrig öppnas igen.

## Testning

- **28 unit tests** (bokforing-core) — domänlogik, balansvalidering, hashkedja, periodmaskin
- **9 integrationstester** (bokforing-db) — fullständigt bokföringsflöde, rättelser, CRUD, E2E-acceptans
- **4 exporttester** — SIE4-generering, CSV-output, CP437-kodning

```bash
cargo test --workspace
# Kör specifik testsvit
cargo test -p bokforing-db --test integration_test
cargo test -p bokforing-core
```
