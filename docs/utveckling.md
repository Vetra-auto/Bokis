# Utveckling

## Bygga

```bash
cargo build --release -p bok-cli -p bok-mcp -p bok-review -p bok-fjarr
cargo test -p bok-store                  # och så vidare, en crate i taget
```

## Crates

| Crate | Vad den gör |
| --- | --- |
| `bok-core` | Domänen: belopp i öre, verifikationer, signaturer och säkerhetsnivåer |
| `bok-store` | SQLite-lagret: spärrarna i schemat, den hashkedjade historiken, rapporter och moms |
| `bok-cli` | `bok`, kommandoraden för den som granskar och bokför |
| `bok-archive` | Underlagsarkivet: en fil per SHA-256 i `underlag/<ab>/<hash>`, skrivskyddad, kontrolleras vid varje läsning. Filtyp avgörs av innehållet |
| `bok-intake` | Läser en bevakad mapp: väntar tills filen är färdigskriven, hoppar över dolda och halvt nedladdade filer |
| `bok-backup` | Kopia med `VACUUM INTO`, delad underlagspool på målet, manifest med SHA-256, läs-in bara till tom katalog |
| `bok-bank` | camt.053 (.02 och .08) och CSV till bankrader i öre med stabila id och saldokontroll |
| `bok-mcp` | MCP-servern. Agentens verktyg: lista och hämta underlag, kontoplan, regler, tidigare kontering, föreslå. Förslaget sparas med härkomst och argumentens SHA-256 som idempotensnyckel |
| `bok-review` | Granskningsvyn: engångsnyckel i inloggningslänken, CSRF-skydd, CSP, underlag visas aldrig som aktivt innehåll |
| `bok-eval` | Mäter agentens konteringar mot facit i Accounted Bench genom MCP-servern |
| `bok-insyn` | Insynen för revisor och Skatteverket: bokföringen som läsbara sidor, live på 127.0.0.1 eller som fristående export |
| `bok-inventarier` | Inventarieregistret som ren logik: avskrivning enligt plan per månad, avyttring, halva prisbasbeloppet och treårsregeln |
| `bok-sru` | SRU-filer för Skatteverkets filöverföring: `INFO.SRU` och `BLANKETTER.SRU` i ISO 8859-1, och en sträng läsare för kontroll |
| `bok-deklaration` | Inkomstdeklaration 2 (INK2, INK2R, INK2S) ur saldobalansen. Fältförteckningen och kontokopplingen är data, och bolagsskatten räknas här |
| `bok-arkivplan` | Arkivplanen som data: delarna (original, säkerhetskopia, arkivpaket, papper), kontrollen av filen, iakttagelser mot BFL och BFNAR och bevarandetiden |
| `bok-bokslut` | Bokslutsposterna som förslag i serie B: avskrivningar, avyttring, periodisering och bolagsskatt, och deklarationens indata ur lagret |
| `bok-fjarr` | Bokis på distans: agenten ansluter över HTTPS med OAuth och kopplingskod, och telefonen skickar kvittofoton med en personlig länk |

## Hela flödet

`scripts/e2e-ai-first.sh <ny katalog>` kör hela flödet med en engångsnyckel: inkorg, agentens
förslag och fråga via `bok-mcp`, godkännande i `bok-review`, svar på frågan, kontoutdrag och
agentens betalningsförslag, avstämning, stängd period, verifiering, kopia och läs-in-övning.

`scripts/e2e-bokslut-ink2.sh <ny katalog>` provar bokslutet i en provbokföring: verifikationer,
inventarieregistret, agentens avskrivningsförslag genom `bok-mcp`, periodisering med återföring,
bolagsskatt, årets resultat, INK2 som SRU-filer, insynen och en full verifiering.

`scripts/e2e-checklista.sh <ny katalog>` provar checklistan från 2026-11-17: organisationsnummer
med fel kontrollsiffra stoppas, motpartens adress, vinstmarginal och trepartshandel i
momsrapporten, arkivplanen i systemdokumentationen, insyn per person mot en riktig insynstjänst
(inloggning loggas, återkallelse stänger sessionen) och MCP 2026-07-28 mot `bok-mcp`.

## Schema

Schemaversion 2 (`schema_v2.sql`) lägger till `bankrader`, `bankmatchningar`,
`matchningsforslag`, `arenden` och `arendesvar`, som alla bara kan växa. En databas med version 1
uppgraderas när den öppnas.

Schemaversion 3 (`schema_v3.sql`) lägger till `sakerhetsniva`, besluten om säkerhetsnivå.

Schemaversion 4 (`schema_v4.sql`) lägger till inventarieregistret, `inventarier` och
`inventarieavgangar`, som bara kan växa. En felregistrering rättas med en avgång av typen
`felregistrering` och en ny registrering.

Schemaversion 5 (`schema_v5.sql`) lägger till motpartens adress: `motparter.adress` och
`motpart_adress` på `forslag` och `verifikationer`, med `ALTER TABLE ADD COLUMN`. Spärrarna från
version 1 gäller hela raden. Adressen ingår i verifikationens kanoniska form bara när den finns,
så signaturer från före version 5 verifieras som förut. Arkivplanen och insynslänkarna har inga
egna tabeller; de är händelser i behandlingshistoriken (`arkivplan.satt`, `insyn.lank.skapad`,
`insyn.lank.aterkallad`, `insyn.inloggning`).

## Granskningar

En fristående granskning av nattens kod hittade sex brister, som alla är åtgärdade och har
tester: en avstämning kunde ersättas med `INSERT OR REPLACE`, ett underlag som dömts ut som
dubblett kunde ändå bokföras, bankrader i annan valuta och på 0 kr kunde stämmas av fel, agentens
text kunde innehålla styrtecken som ändrar vad terminalen visar, momskoder föll bort vid ny
inläsning av kontoplanen, och granskningsvyns inloggningslänk gick att återanvända.

En kontroll i en riktig webbläsare hittade ytterligare ett fel: med `Referrer-Policy: no-referrer`
skickar webbläsare `Origin: null` på formulär, så Origin-kontrollen nekade varje POST, även
godkännandet. Testerna och e2e-skriptet skickade Origin själva och märkte inget. Nu är policyn
`same-origin`, och ett test låser det.
