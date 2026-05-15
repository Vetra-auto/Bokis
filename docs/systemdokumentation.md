# Systemdokumentation — Bokis

**Enligt BFNAR 2013:2 (Bokföringsnämndens allmänna råd om bokföring)**

Dokumentversion: 1.0  
Datum: 2026-05-14  
System: Bokis 0.1.0  
Ansvarig: Systemadministratör

---

## 1. Allmän systembeskrivning

### 1.1 Systemets namn och version
- **Namn:** Bokis
- **Version:** 0.1.0
- **Typ:** Lokal desktop-applikation (local-first)
- **Plattform:** Linux (x86_64), paketerad som .deb/.AppImage

### 1.2 Syfte
Bokis är ett bokföringssystem avsett för enskilda firmor och mindre aktiebolag. Systemet hanterar löpande bokföring, verifikationer, periodstängning, export och arkivering i enlighet med:
- Bokföringslagen (1999:1078)
- BFNAR 2013:2 (Bokföring)
- BAS-kontoplanen (BAS 2024)

### 1.3 Teknisk arkitektur
| Komponent | Teknologi |
|-----------|-----------|
| Applikation | Tauri 2 (Rust) |
| Frontend | Leptos (Rust → WebAssembly) |
| Databas | SQLite 3 med WAL-läge |
| Kryptering lösenord | Argon2id |
| Integritetsskydd | SHA-256 hashkedja |
| Exportformat | SIE4, CSV, JSON, PDF |

Systemet kräver ingen internetanslutning och lagrar all data lokalt.

---

## 2. Datalagring

### 2.1 Lagringsplats
All data lagras i användarens lokala applikationskatalog:
```
~/.local/share/se.bokforing.app/
├── bokforing.db          # Huvuddatabas
├── attachments/          # Verifikationsbilagor
├── exports/              # Exporterade rapporter
├── backups/              # Säkerhetskopior
└── archives/             # Årsarkiv
```

### 2.2 Databasformat
- SQLite 3 med WAL (Write-Ahead Logging) för konsistens vid strömavbrott
- Alla belopp lagras som heltal i öre (INTEGER) — inga avrundningsfel
- UUID som primärnycklar (TEXT)
- Tidsstämplar i ISO 8601-format (TEXT)
- Foreign keys aktiverade (`PRAGMA foreign_keys = ON`)

### 2.3 Tabellöversikt

| Tabell | Innehåll |
|--------|----------|
| `companies` | Företagsuppgifter (namn, org.nummer) |
| `users` | Användare med Argon2id-hashade lösenord |
| `role_assignments` | Rollkopplingar (admin/bokförare/attestant/revisor) |
| `fiscal_years` | Räkenskapsår (start, slut, stängd-flagga) |
| `periods` | Bokföringsperioder (månadsvisa, med status) |
| `accounts` | Kontoplan med kontonummer, namn, typ, SRU-kod |
| `counterparties` | Motparter (leverantörer, kunder) |
| `vouchers` | Verifikationer (serie, nummer, datum, status) |
| `voucher_lines` | Verifikationsrader (konto, debet, kredit) |
| `attachments` | Bilagor med SHA-256 filhash |
| `correction_links` | Koppling original→reversering→ersättning |
| `audit_events` | Revisionslogg med hashkedja |
| `processing_history_events` | Behandlingshistorik |
| `archive_manifests` | Arkiveringsprotokoll |
| `backup_jobs` | Backup-loggar |
| `system_documentation_snapshots` | Versioner av systemdokumentation |

---

## 3. Behandlingsregler

### 3.1 Verifikationer

**Skapande (utkast):**
- Verifikation skapas med status `draft`
- Kräver: minst 2 rader, datum inom öppen period, giltiga konton
- Serienummer tilldelas automatiskt per serie (A, B, etc.) och räkenskapsår
- Balanscheck: summa debet = summa kredit (kontrolleras före bokföring)

**Bokföring:**
- Status ändras från `draft` till `posted`
- Tidsstämpel (`posted_at`) och användare registreras
- **Oåterkallelig** — en bokförd verifikation kan aldrig ändras eller raderas

**Rättelse (BFL 5 kap. 5§):**
1. Systemet skapar en reverseringsverifikation (debet↔kredit spegelvändas)
2. Användaren skapar en ersättningsverifikation med korrekta belopp
3. `correction_links`-tabellen kopplar: original → reversering → ersättning
4. Motivering (fältet `reason`) är obligatorisk
5. Rättelsen loggas i revisionsloggen

### 3.2 Perioder

Varje räkenskapsår delas i 12 månadsperioder med följande livscykel:

```
open → under_review → closed → locked
  ↑         |
  +---------+  (reopen — kräver admin + loggning)
```

- **open** — Bokföring tillåten
- **under_review** — Granskning pågår, ingen ny bokföring
- **closed** — Stängd, kan öppnas igen av admin med motivering
- **locked** — Permanent stängd, kan aldrig öppnas igen

Bokföring i en period som inte har status `open` nekas av systemet.

### 3.3 Räkenskapsår

- Definieras med start- och slutdatum
- Kan stängas (`is_closed = true`) när alla perioder är stängda/låsta
- Stängt räkenskapsår kan arkiveras

### 3.4 Pengar och belopp

- Alla belopp representeras internt som heltal i **öre** (1 kr = 100 öre)
- Datatyp: 64-bitars heltal (i64)
- Exempel: 10 050 öre = 100,50 kr
- Inga flyttalsberäkningar — eliminerar avrundningsfel

---

## 4. Kontroller och säkerhet

### 4.1 Behörighetssystem

| Roll | Behörighet |
|------|-----------|
| admin | Full åtkomst: användare, räkenskapsår, periodlåsning, arkivering |
| bokforare | Skapa/bokföra verifikationer, rätta poster |
| attestant | Granska och godkänna verifikationer |
| revisor | Läsbehörighet + hashkedjeverifiering |

Rollkontroll sker i varje kommando (Tauri command layer).

### 4.2 Autentisering

- Lösenord hashas med **Argon2id** (minneskostsam hashfunktion)
- Sessionshantering per användare
- Tvingat lösenordsbyte vid första inloggning (`force_password_change`)

### 4.3 SHA-256 revisionskedja

Varje händelse i `audit_events` hashas enligt:
```
event_hash = SHA-256(
    sequence_number | event_type | entity_id | occurred_at | detail_json | previous_hash
)
```
- Genesishash: `"0" × 64` (64 nollor)
- Kedjan kan verifieras via användargränssnittet (admin/revisor)
- En bruten kedja indikerar att data har manipulerats utanför systemet

### 4.4 Bilagor

- Filer lagras med SHA-256 hash av innehållet
- MIME-typ detekteras automatiskt
- Lagringsväg: `attachments/{verifikations-id}/{bilage-id}.{ext}`

---

## 5. In- och utmatning

### 5.1 Inmatning

- Manuell registrering via grafiskt gränssnitt (desktop-app)
- Verifikationer skapas rad för rad med konto, debet/kredit, beskrivning
- Bilagor kan bifogas via filväljare

### 5.2 Utmatning / Export

| Format | Innehåll | Användning |
|--------|----------|-----------|
| SIE4 | Fullständig bokföringsdata (CP437-kodning) | Import i andra bokföringsprogram |
| CSV | Grundbok (semikolonseparerad, UTF-8) | Kalkylblad (Excel, LibreOffice) |
| JSON | Komplett export inkl. revisionslogg | Digital arkivering |
| PDF | Grundbok eller balansrapport | Utskrift, fysisk arkivering |

### 5.3 Rapporter (via UI)

- **Grundbok** — Alla bokförda verifikationer i datumordning
- **Huvudbok** — Kontovisa transaktioner med löpande saldo
- **Balansrapport** — Summering per konto (debet/kredit/saldo)
- **Revisionslogg** — Alla systemhändelser med hashkedja

---

## 6. Arkivering och retention

### 6.1 Krav (BFL 7 kap. 2§)

Räkenskapsinformation ska bevaras i minst **7 år** efter utgången av det kalenderår då räkenskapsåret avslutades.

### 6.2 Arkiveringsprocess

1. Räkenskapsåret måste vara stängt (`is_closed = true`)
2. Systemet skapar en komplett databaskopia via `VACUUM INTO` (konsistent snapshot)
3. SHA-256 hash beräknas på arkivfilen
4. Arkivmanifest sparas med:
   - Räkenskapsår
   - Antal konton och bokförda verifikationer
   - Filväg och hash
   - Arkiveringsdatum
   - Retentionstid (7 år)
5. Händelsen loggas i revisionsloggen

### 6.3 Arkivformat

Arkivet är en fullständig SQLite-databas som kan öppnas oberoende av Bokis-applikationen med valfritt SQLite-verktyg (t.ex. `sqlite3` CLI).

### 6.4 Backup

- **Metod:** `VACUUM INTO` (skapar konsistent kopia trots WAL-läge)
- **Verifiering:** `PRAGMA integrity_check` + SHA-256 hashkontroll
- **Lagring:** Lokal katalog (`backups/`)
- **Rekommendation:** Kopiering till extern media (USB, NAS) ska ske manuellt

---

## 7. Systemdokumentation (denna fil)

### 7.1 Automatgenererad dokumentation

Systemet kan generera en maskinläsbar JSON-snapshot av aktuell konfiguration via funktionen "Generera systemdokumentation" i arkivmodulen. Denna innehåller:
- Systemversion
- Företagsuppgifter
- Kontoplan (antal, standard)
- Räkenskapsår (med status)
- Användare (utan lösenord)
- Antal arkiv och backuper
- Retentionspolicy
- Beskrivning av revisionskedja

### 7.2 Versionshantering

Varje genererad snapshot sparas i `system_documentation_snapshots` med:
- Genereringsdatum
- SHA-256 hash av innehållet
- Fullständigt JSON-innehåll

---

## 8. Ändringslogg

| Datum | Version | Beskrivning |
|-------|---------|-------------|
| 2026-05-14 | 1.0 | Initial systemdokumentation |

---

## 9. Kontaktuppgifter

- **Systemansvarig:** [Fyll i namn]
- **Leverantör:** Egenutvecklat system
- **Support:** [Fyll i kontaktinfo]
