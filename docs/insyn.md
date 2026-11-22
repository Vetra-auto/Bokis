# Insyn för revisor och Skatteverket

Revisorn eller Skatteverket ska kunna se hur bokföringen och årsbokslutet ser ut utan att
installera något och utan att kunna ändra något. `bok insyn` visar bokföringen som en pärm med
flikar; samma sidor kan lämnas över som en fristående kopia.

```bash
bok insyn                                 # live på 127.0.0.1; länken som skrivs ut fungerar en gång
bok insyn export /media/usb/bokforing     # hela bokföringen som fristående filer
bok insyn export /media/usb/bokforing-2026 --year 2026   # ett räkenskapsår
```

Som tjänst (till exempel bakom `tailscale serve`) får varje person en egen länk:

```bash
bok insyn --port 8790 --adress https://bokis.ditt-tailnet.ts.net:8443   # tjänsten, utan startlänk
bok insyn lank --namn "Revisor X" --giltig 30d   # länk till en person; 15m, 12h eller 30d, högst 366d
bok insyn lankar                                 # vem som har länk, giltig till, inloggningar, senast
bok insyn aterkalla 12                           # länk 12 slutar gälla direkt, också öppna sessioner
```

En länk kan användas flera gånger tills den går ut eller återkallas. Varje inloggning ger en
session som gäller högst 12 timmar och aldrig längre än länken, och loggas i
behandlingshistoriken (`insyn.inloggning`, med personens namn och sessionens sluttid) innan
kakan ges; går det inte att logga blir det ingen inloggning. Att länken skapas och återkallas
loggas också. Historiken avgör om en länk gäller: tjänsten frågar vid varje sida, så en
återkallelse gäller direkt. `insyn.json` bredvid databasen sparar bara SHA-256 av länkar och
sessioner. Utan `--namn` får länken ditt namn och gäller 15 minuter.

| Flik | Innehåll |
| --- | --- |
| Översikt | Kontrollstämpeln: hela `bok verify` (kedjan, serierna, varje signatur, perioderna, banken, underlagen i arkivet) med antal kontrollerade poster, räkenskapsåren med resultat och perioder, nycklarna och hur bokföringen förs |
| Grundbok | Verifikationerna i registreringsordning med konteringen, rättelser åt båda hållen och en CSV för kalkylprogram |
| Verifikation | Blanketten: datum, sammanställningsdag, registrering, motpart, kontering, underlaget bredvid (bild, PDF eller text), signaturen och hasharna, agentens förslag med modell, säkerhet och fullständigt svar, beslutet och alla händelser som rör den |
| Huvudbok, saldobalans | Per konto med ingående balans, löpande saldo och utgående balans; kontonumren länkar mellan sidorna |
| Årsbokslut | Resultat- och balansräkning efter BAS-grupperna med föregående år, och en bokslutskontroll: perioder stängda, bankrader avstämda, resultatet bokfört, balansräkningen balanserar, ingående balans mot föregående års utgående, ingående balans för nästa år, kontoavstämningar på bokslutsdagen |
| Moms | Rutorna för året med kontona bakom, och per månad |
| Bank | Bankraderna och vad var och en stämdes av mot, och kontoavstämningarna |
| Underlag | Alla mottagna underlag med status och verifikation, även dubbletter och sådant som inte bokförts |
| Behandlingshistorik | Varje händelse med vem, roll, vad, när, gamla och nya värden och hash |
| Systemdokumentation, Export | Samma systemdokumentation som `bok systemdokumentation`, och hur kopian kontrolleras |

Exporten innehåller sidorna, underlagen byte för byte, SIE 4 per år, grundboken som CSV och
`manifest.sha256`, och kontrolleras med `sha256sum -c manifest.sha256` (exportsidan har kommandot
för macOS och Windows också). Sidorna har inget JavaScript, inga formulär och inga externa
resurser, så de fungerar från ett USB-minne utan nät och skrivs ut som de ser ut. En export
loggas i behandlingshistoriken, och en befintlig katalog skrivs aldrig över.

Liveläget har samma skydd som granskningsvyn men tar bara emot GET: Host-kontroll, länk
till en sessionskaka (`bok_insyn`, eftersom kakor inte skiljer på portar), CSP utan skript.
Underlag får en filändelse efter sitt innehåll, aldrig `.html` eller `.svg`; det webbläsaren inte
kan visa skickas som nedladdning. Text från underlag och agenten escapas, och styrtecken och
riktningsmarkörer visas som `\u{…}`.

`python3 scripts/demo-insyn.py <ny katalog>` bygger en påhittad bokföring att visa insynen med:
ett stängt år 2025 med bokslut och ingående balans, ett pågående år 2026 med kvitton som bilder,
agentförslag genom `bok-mcp`, en rättelse, bank, momsavräkning, ett dubblettkvitto och en
obesvarad fråga. Kräver Pillow.
