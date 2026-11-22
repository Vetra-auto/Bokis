# Använda Bokis

Alla kommandon i `bok`, säkerhetsnivåerna och hur bank, moms, bokslut, inkomstdeklaration och
agenten hänger ihop.

## Bokföra med `bok`

`bok` är verktyget för den som granskar och bokför. Ett förslag får inget verifikationsnummer;
numret tas ur serien först när du godkänner, och varje bokföring signeras med din nyckel
(`ssh-keygen -Y sign`, helst en YubiKey med en `ed25519-sk`-nyckel). Ett avvisat förslag
förbrukar inget nummer. Bokförda verifikationer, rader, underlag och historiken kan inte ändras
eller raderas; databasen själv vägrar. Fel rättas med en ny verifikation i serie R.

```bash
cargo build --release -p bok-cli          # ger target/release/bok
ssh-keygen -t ed25519-sk                  # en gång: nyckel på YubiKeyn, ger ~/.ssh/id_ed25519_sk
```

Organisationsnumret kontrolleras när bokföringen skapas och när en motpart anges: tio siffror
(NNNNNN-NNNN) med rätt kontrollsiffra, eller personnummer för en enskild firma. Numret sparas på
formen NNNNNN-NNNN även om det skrevs utan bindestreck. Ett utländskt nummer skrivs med landskoden
först, till exempel `DK12345678`; det kan Bokis inte kontrollera. Företagets uppgifter kan inte
ändras efteråt, så ett felskrivet nummer stoppas innan något sparas.

Databasen ligger i `~/.local/share/bokis/bokis.db` och underlagen i katalogen `underlag/` bredvid.
Ange annan plats med `--db` eller `BOKIS_DB`, och annan nyckel med `--key` eller `BOKIS_KEY`.

```bash
# Ny bokföring: företaget, din nyckel och första räkenskapsåret (A, B, I och R börjar på 1)
bok init --company "Exempel AB" --orgnr 559000-0005 --year 2026
bok account import kontoplan.se           # #KONTO ur en SIE-fil, eller KONTO;NAMN per rad

# Underlag och förslag
bok attachment add kvitto.pdf             # sparas oförändrat med SHA-256 och mottagningstid
bok propose --date 2026-03-15 --text "Papper" --line 5410=80,00 --line 2641=20,00 \
    --line 1930=-100,00 --attachment 1 --counterparty-name "Papper AB" --orgnr 556000-0001
bok proposals                             # förslag som väntar på beslut
bok approve 1                             # bokför som nästa nummer i serien; rör vid nyckeln
bok reject 2 --reason "dubblett"

# Rättelse: ny verifikation i serie R som tar ut originalet (och lägger till rätt rader)
bok correct A1 --reason "fel konto" --line 6110=80,00 --line 1930=-80,00 --approve

# Perioder: öppen → under avstämning → stängd → låst. Stängd och låst öppnas aldrig igen.
bok period review 2026-03 && bok period close 2026-03 && bok period lock 2026-03

# Rapporter, kontroll och export
bok journal                               # grundbok i registreringsordning
bok ledger --account 1930                 # huvudbok med ingående balans
bok trial-balance                         # saldobalans
bok show A1                               # verifikationen med signatur och hashar
bok history                               # behandlingshistoriken
bok verify                                # serier, historikens kedja, signaturer, arkivet
bok export sie --out bokis-2026.se        # SIE 4 med #IB, #UB och #RES
```

Positivt belopp i `--line` är debet och negativt kredit. Ett förslag kan också läggas som JSON
med `--file` (samma form som agenten kommer att använda). Kommandona har svenska alias:
`godkann`, `avvisa`, `ratta`, `grundbok`, `huvudbok`, `saldobalans`, `verifiera` med flera.
Det finns inga kommandon som raderar, nollställer eller återställer en databas, och `bok init`
skriver aldrig över en befintlig fil.

## Säkerhetsnivå

Bokföringslagen kräver ingen viss sorts signatur. Den kräver att det syns vem som gjorde vad och
när, och att bokföringen inte kan ändras i efterhand utan att det märks. Det klarar Bokis på alla
tre nivåerna. Nivån avgör hur svårt det är för någon annan, till exempel en AI-agent på samma
dator, att godkänna något i ditt namn.

| Nivå | Bokis godtar | Vem kan signera i ditt namn |
|------|--------------|-----------------------------|
| `mjukvara` | alla registrerade nycklar, också en nyckelfil | allt som kör som du och når nyckeln |
| `hardvara` (standard) | bara hårdvarunycklar, som måste röras vid varje signatur | bara den som rör nyckeln |
| `hardvara-pin` | som `hardvara`, och nyckelns PIN vid varje signatur | bara den som rör nyckeln och kan PIN-koden |

```bash
ssh-keygen -t ed25519-sk                       # hardvara: nyckeln på YubiKeyn
ssh-keygen -t ed25519-sk -O verify-required    # hardvara-pin: YubiKeyn kräver också PIN
ssh-keygen -t ed25519                          # mjukvara: en nyckelfil, gärna med lösenfras

bok init --security mjukvara --company "Exempel AB" --orgnr 559000-0005 --year 2026
bok sakerhet visa                              # nivån och alla beslut om den
bok sakerhet satt hardvara-pin                 # signeras; beslutet står i historiken
```

Den som höjer nivån signerar med en nyckel som klarar den nya nivån, och den som sänker den med en
nyckel som klarar den gamla. En agent eller någon som kommit åt en nyckelfil kan alltså inte sänka
nivån. Registrera en reservnyckel med `bok nyckel lagg-till` innan du höjer. Utan en nyckel som
klarar nivån går det inte att signera något alls.

På nivån `mjukvara` gör en lösenfras på nyckelfilen stor skillnad, men den syns inte i signaturen.
Bokis kan därför inte kräva den. Ha nyckeln i ssh-agent bara när du godkänner, till exempel med
`ssh-add -t 10m ~/.ssh/id_ed25519`, och peka ut den med `BOKIS_KEY`.

Bokis prövar det som signaturen visar: nyckeltypen och om nyckeln intygade beröring och PIN. Att
en hårdvarunyckel verkligen sitter i en YubiKey förutsätter att det var du som registrerade den. En
bokföring från före nivåerna har inget beslut och godtar alla registrerade nycklar tills du
beslutar en nivå.

## Inkorg, kopior, bank, moms, bokslut och agent

```bash
cargo build --release -p bok-cli -p bok-mcp -p bok-review

# Inkorg: nya filer arkiveras oförändrade, registreras och flyttas sedan till <mapp>/inlasta
bok inkorg ~/Kvitton

# Kopior till en extern plats (BFNAR 2013:2 p. 8.2), med SHA-256-manifest
bok kopia skapa /media/usb/bokis          # konsistent kopia av databasen och nya underlag
bok kopia lista /media/usb/bokis
bok kopia kontrollera /media/usb/bokis/kopior/<tid>
bok kopia las-in /media/usb/bokis/kopior/<tid> /tmp/provlasning   # bara till en tom katalog

# Bank: kontoutdraget arkiveras som underlag och raderna läses in (en gång, även om filen läses igen)
bok bank importera utdrag.xml             # camt.053; CSV med --csv-account <konto>; --account 1930
bok bank importera transactions-….csv --csv-account <kontonummer>   # Lunar Business, se nedan
bok bank rader                            # rader som inte är avstämda (--all för alla)
bok bank forslag                          # verifikationer med samma belopp, och agentens förslag
bok bank matcha 1 A12                     # stäm av rad 1 mot A12 (beloppet på 1930 måste stämma)
bok bank matcha 2 --explanation "..."     # eller med en skriftlig förklaring
# En period med bankrader som inte är avstämda kan varken stängas eller låsas.

# Moms: momskoden på kontot är rutan i momsdeklarationen, och den är data i kontoplanen.
bok konto importera kontoplan.se --vat-codes docs/momskoder-bas.txt   # KONTROLLERA filen först
bok moms 2026-03                          # eller 2026-01:2026-03, eller 2026
bok konto momskoder ändringar.txt --from 2026-07-01                   # ändringar gäller framåt

# Avstämning mot ett externt saldo, till exempel skattekontot mot Skatteverket; loggas i historiken
bok avstam 1630 --date 2026-03-31 --balance 12345,00 --note "Skattekontot mars"

# Systemdokumentation, genererad ur bokföringen: spärrarna som de står i schemat, agentens och
# granskarens operationer, nycklar, perioder, momskoder och en full verifiering. Loggas i historiken.
bok systemdokumentation systemdokumentation-2026-11.md

# Arkivplan (BFNAR 2013:2 p. 8.3 och 9.2 d): vad som arkiveras, var, hur ofta och hur det
# kontrolleras. Planen är din egen JSON-fil (mall: docs/arkivplan-exempel.json) och läggs in som en ny
# version i historiken; systemdokumentationen och insynen visar den med bevarandetid per år och den
# senaste kopian till varje plats som har en "sokvag".
bok arkivplan satt arkivplan.json
bok arkivplan visa

# Bokslut: förslag som du granskar och bokför med nyckeln som alla andra (se Bokslut nedan)
bok bokslut avskrivningar --year 2026     # serie B: årets avskrivningar ur inventarieregistret
bok bokslut skatt --year 2026             # serie B: bolagsskatten ur deklarationens överskott
bok bokslut resultat --year 2026          # serie B: årets resultat från 8999 till 2099 (--equity-account 2019 för EF)
bok ar lagg-till 2027
bok bokslut ib --year 2026                # serie I i 2027: balanskontonas utgående saldon
bok arkivpaket /media/usb/arkiv --year 2026   # SIE, rapporter, underlag, historik, systemdok, programmet
                                          # kontrolleras utan Bokis: sha256sum -c manifest.sha256

# Regler som agenten följer (hamta_regler). Varje version signeras och gäller från ett datum.
bok regel lagg-till molntjanster --text "Molntjänster: 5420, ingående moms 25 % på 2641." --from 2026-01-01
bok regel lista                           # som de gäller i dag (--date, --all)

# Agentens frågor och flaggor blir ärenden som du besvarar. Bara ditt svar på en flagga
# kan ändra ett underlags status, och aldrig för ett bokfört underlag.
bok arende lista                          # obesvarade (--all för alla)
bok arende svara 3 "Kund X och jag"
bok arende svara 4 "Reklam" --mark ej-rakenskapsinformation   # eller --mark dubblett

# Agenten: MCP-server över stdio. Den kan läsa och föreslå men inte bokföra.
claude mcp add bokis -- ~/bokforing/target/release/bok-mcp --db ~/.local/share/bokis/bokis.db

# Granskningsvyn på 127.0.0.1: förslaget bredvid underlaget, godkänn signerar med nyckeln.
# Sidorna Bank och Ärenden: stäm av bankrader mot föreslagna verifikationer och svara på agentens
# frågor och flaggor. Länken som skrivs ut fungerar en gång.
bok-review --db ~/.local/share/bokis/bokis.db
```

## Kontoutdrag från Lunar

Lunar Business har ingen camt.053. Exportera i stället CSV under kontots Detaljer, "Exportera
kontoutdrag", med filformatet CSV och utan bilagor. Bokis känner igen filen på rubrikraden, läser
raderna i tidsordning (Lunar lägger den senaste först) och kontrollerar saldot med radernas saldon.
Samma rad känns igen på Lunars transaktions-id, så exporter som överlappar ger inga dubbletter.

## Moms

Momsrapporten summerar bokförda rader per momskod med blankettens tecken och räknar fram ruta
49. Verifikationer med en rad på ett konto med momskod 49 (2650) är momsavräkningar och räknas
inte med. [`momskoder-bas.txt`](momskoder-bas.txt) är granskad mot BAS 2026 och Skatteverkets
beskrivning av fälten, se [lagkontrollen](lagkontroll.md). Stäm ändå av den mot din egen
kontoplan.

## Bokslut: inventarier, periodiseringar och skatt

Bokslutsposterna blir förslag i serie B, daterade räkenskapsårets sista dag, som du granskar och
bokför med `bok godkann` som alla andra. Bokis räknar beloppen; ingenting bokförs utan din
signatur, och samma förslag läggs inte två gånger. Ordningen nedan är den naturliga: avskrivningar
och periodiseringar först, sedan skatten och sist årets resultat.

```bash
# Inventarieregistret. Anskaffningen bokförs som vanligt (till exempel 1220), sedan registreras den.
bok inventarie lagg-till --name "Bärbar dator" --date 2026-05-12 --amount 36000 --life 36 --voucher A2
bok inventarie lista --year 2026          # bokfört värde enligt plan och avstämning mot huvudboken
bok inventarie plan 1                     # avskrivning och bokfört värde år för år
bok inventarie avyttra 1 --date 2027-03-15 --kind forsaljning --price 20000 --vat 5000
bok inventarie avyttra 2 --date 2026-05-12 --kind felregistrering --note "Dras av direkt"

# Årets avskrivningar enligt plan (debet 7832, kredit 1229 per inventarium)
bok bokslut avskrivningar --year 2026

# Periodisering: förutbetald-kostnad, upplupen-kostnad, forutbetald-intakt eller upplupen-intakt.
# --aterfor lägger också återföringen på nästa års första dag.
bok bokslut periodisera --year 2026 --kind forutbetald-kostnad --account 6310 --amount 12000 \
    --from 2026-11-01 --to 2027-10-31 --text "Företagsförsäkring" --aterfor

# Bolagsskatten ur deklarationens överskott (debet 8910, kredit 2512)
bok bokslut skatt --year 2026             # --falt 4.14a=150000 för underskott från tidigare år

bok bokslut resultat --year 2026          # sist: årets resultat till eget kapital
```

**Inventarier.** Avskrivningen är linjär enligt plan: lika mycket varje månad från den månad
inventariet tas i bruk (`--from 2026-06` om det inte är anskaffningsmånaden), som K2 tillåter.
K2 godtar fem år, 60 månader, som nyttjandeperiod för inventarier, men då utan restvärde
(`--residual`; K2 punkt 10.25 och 10.27). Ett inventarium vars
anskaffningsvärde utan moms understiger ett halvt prisbasbelopp (29 600 kr för 2026), eller vars
livslängd är högst tre år, får kostnadsföras direkt (det är prisbasbeloppet för året då
beskattningsåret går ut som gäller); `bok inventarie lagg-till` säger till.
Prisbasbeloppen finns i `crates/bok-inventarier/data/prisbasbelopp.txt` och läggs till varje år.
Registret kan bara växa: ett inventarium som registrerats fel stryks med en avgång av typen
`felregistrering` och registreras på nytt. Vid försäljning och utrangering skrivs inventariet av till
och med månaden före avgången, och förslaget bokför vinsten eller förlusten (3973 eller 7973).
Momsen på en försäljning anges från fakturan med `--vat`; Bokis räknar aldrig fram den.

**Periodisering.** Andelen räknas per dag. En utgift eller inkomst under 7 000 kr per faktura eller
avtal behöver inte periodiseras enligt K2 (5 000 kr för räkenskapsår som började före 2026);
`--anda` periodiserar ändå.

**Skatt.** Överskottet avrundas nedåt till helt tiotal kronor och skatten är 20,6 % av det. Är
skatten redan bokförd föreslås bara skillnaden.

## Inkomstdeklaration 2 som SRU-filer

`bok deklaration ink2` räknar fram INK2, INK2R och INK2S ur saldobalansen och skriver filerna för
Skatteverkets filöverföring till en ny katalog:

```bash
bok deklaration ink2 --year 2026 ~/deklaration-2026 --postnr 12345 --postort Småstad \
    --kontakt "Kim Exempel" --falt 8041=X --falt 8045=X
# ger INFO.SRU, BLANKETTER.SRU och ink2-sammanstallning.md
```

- **Blanketten** följer räkenskapsårets sista dag: ett år som slutar 2026-12-31 blir `INK2-2026P4`,
  `INK2R-2026P4` och `INK2S-2026P4`. Fältförteckningen är avskriven ur Skatteverkets tabeller
  (`crates/bok-deklaration/data/ink2-2026p4.txt`); en ny period kräver en ny fil.
- **INK2R** (balans- och resultaträkning) följer [`ink2r-bas.txt`](ink2r-bas.txt), BAS
  kopplingstabell, eller kontots egen SRU-kod i kontoplanen (`#SRU` i en SIE-fil). Ett konto med
  saldo som inte hör till något fält stoppar deklarationen. Årets resultat som inte är bokfört mot
  eget kapital läggs till fritt eget kapital (2.28).
- **INK2S** börjar med årets resultat och skatten, och lägger tillbaka kostnader som inte får dras
  av och drar av skattefria intäkter enligt [`ink2s-bas.txt`](ink2s-bas.txt). Det som inte syns i
  bokföringen anges med `--falt FÄLT=BELOPP` i hela kronor, till exempel `--falt 4.14a=150000`
  (outnyttjat underskott) eller `--falt 4.6a=…` (schablonintäkt på periodiseringsfond). Kryssrutorna
  om revision och redovisningskonsult anges med `--falt 8045=X` och `--falt 8041=X`.
- **Kontroller**: balansräkningen balanserar, resultaträkningen summerar till årets resultat,
  resultatet och överskottet förs vidare till rätt fält, varje belopp ryms i fältets datatyp och
  fält som utesluter varandra är inte ifyllda samtidigt. Filerna skrivs bara när allt går igenom,
  läses tillbaka och prövas mot fältförteckningen.
- Filerna skrivs aldrig över och loggas i behandlingshistoriken med SHA-256. Utkastet syns också
  under fliken Deklaration i insynen, och inventarieregistret under fliken Inventarier.

Granska sammanställningen mot årsredovisningen innan du lämnar in. Bokis gör inte
årsredovisningen, och fält som kräver bedömning (periodiseringsfond, koncernbidrag, underskott,
fastigheter) fylls bara i med `--falt`.
