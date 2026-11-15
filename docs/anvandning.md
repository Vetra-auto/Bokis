# Använda Bokis

Alla kommandon i `bok`, säkerhetsnivåerna och hur bank, moms, bokslut och agenten hänger ihop.

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

Databasen ligger i `~/.local/share/bokis/bokis.db` och underlagen i katalogen `underlag/` bredvid.
Ange annan plats med `--db` eller `BOKIS_DB`, och annan nyckel med `--key` eller `BOKIS_KEY`.

```bash
# Ny bokföring: företaget, din nyckel och första räkenskapsåret (A, B, I och R börjar på 1)
bok init --company "Vetra AB" --orgnr 559000-0000 --year 2026
bok account import kontoplan.se           # #KONTO ur en SIE-fil, eller KONTO;NAMN per rad

# Underlag och förslag
bok attachment add kvitto.pdf             # sparas oförändrat med SHA-256 och mottagningstid
bok propose --date 2026-03-15 --text "Papper" --line 5410=80,00 --line 2641=20,00 \
    --line 1930=-100,00 --attachment 1 --counterparty-name "Papper AB" --orgnr 556000-0000
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

bok init --security mjukvara --company "Vetra AB" --orgnr 559000-0000 --year 2026
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

# Bokslut: förslag som du granskar och bokför med nyckeln som alla andra
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
