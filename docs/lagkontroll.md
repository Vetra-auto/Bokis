# Lagkontroll: vilka lagar och krav Bokis uppfyller

Kontrollerad 2026-11-15 mot lagtexterna på riksdagen.se, BFN:s vägledning Bokföring och
Skatteverkets sidor. Bokslutet och inkomstdeklarationen kontrollerades 2026-11-16 mot
inkomstskattelagen, årsredovisningslagen, BFN:s vägledning K2 och Skatteverkets SRU-format. Lagar och belopp ändras: kontrollera igen minst en gång om året och när
Skatteverket eller BFN ändrar något.

Bokis är ett verktyg. Ansvaret för bokföringen ligger kvar hos företaget, som också lämnar
deklarationer och årsredovisning.

## Kartläggning

| Krav | Lagrum | Så gör Bokis | Läge |
|------|--------|--------------|------|
| En verifikation för varje affärshändelse | BFL 5 kap. 6 § | Varje bokföring är en verifikation med länk till sitt underlag. Ett förslag utan underlag eller bankrad får en varning. | Uppfyllt |
| Verifikationens innehåll: när, vad, belopp, motpart | BFL 5 kap. 7–8 §§, BFNAR 2013:2 p. 5.5–5.7 | Affärshändelsens datum, beskrivning, rader och motpart sparas. Bokis sätter sammanställningsdatum och verifikationsnummer. Saknas motpart får förslaget en varning, eftersom motpart bara får utelämnas vid anonym kontantförsäljning. | Uppfyllt |
| Obruten verifikationsserie | BFNAR 2013:2 p. 5.9 | Numret tas först när förslaget bokförs, i samma transaktion. `bok verify` letar luckor. | Uppfyllt |
| Det bokförda går inte att radera eller ändra | BFL 5 kap. 5 §, BFNAR 2013:2 p. 2.1 och 5.4 | Databasen vägrar ändringar och raderingar (spärrar i schemat), och historiken är en hashkedja. | Uppfyllt |
| Rättelse med rättelsepost i rätt period | BFL 5 kap. 5 §, BFNAR 2013:2 p. 2.17–2.18 | Rättelsen blir en ny verifikation i serie R, kopplad åt båda hållen. Är originalets period stängd bokförs rättelsen i perioden som är öppen nu. | Uppfyllt |
| Grundbok och huvudbok | BFL 5 kap. 1 § | `bok grundbok` i registreringsordning, `bok huvudbok` per konto. | Uppfyllt |
| Behandlingshistorik | BFL 5 kap. 11 § | Varje händelse loggas med vem, när och vad, i en kedja som `bok verify` kontrollerar. | Uppfyllt |
| Systemdokumentation | BFL 5 kap. 11 § | `bok systemdokumentation` skrivs ur databasen: spärrar, operationer, nycklar, perioder, momskoder och kontroll. | Delvis: beskrivningen av arkiveringen (arkivplan) saknas |
| Förvaring i sju år, i Sverige | BFL 7 kap. 2–4 §§ | Databas och underlag ligger på din egen dator. Kopior (`bok kopia`) och arkivpaket per år (`bok arkivpaket`). | Bokis stödjer, du väljer platsen |
| Momsdeklarationens fält | Skatteverkets fält 05–62 (SKV 4700 utg. 22) | `bok moms` räknar fälten ur kontoplanens momskoder. Rubrikerna följer blanketten. | Uppfyllt för konton med momskod; vinstmarginalbeskattning (07) och trepartshandel (37–38) saknas |
| Momsavdrag bara mot faktura eller kvitto som visar momsen | ML 13 kap. 31–32 §§, 17 kap. 24, 26 och 28 §§ | Agentens instruktion och regeln `momsavdrag`. Agenten räknar aldrig fram moms som inte står på underlaget. | Uppfyllt när regeln är inlagd |
| Privata kostnader ger inget avdrag | IL 9 kap. 2 §, ML 13 kap. 6 § | Regeln `privata-kostnader`: agenten frågar i stället för att föreslå en kostnad. | Uppfyllt när regeln är inlagd |
| Representation: moms och inkomstskatt | ML 13 kap. 24–25 §§, IL 16 kap. 2 § | Regeln `representation`: moms på högst 300 kr per person, måltider ej avdragsgilla, enklare förtäring högst 60 kr. | Uppfyllt när regeln är inlagd |
| Gåvor till affärsförbindelser | IL 9 kap. 2 § och 16 kap. 2 §, ML 13 kap. 24 § | Regeln `gavor`: högst 300 kr exklusive moms per person. | Uppfyllt när regeln är inlagd |
| Omvänd betalningsskyldighet och import | ML, Skatteverkets fält 20–24, 30–32, 48, 50 och 60–62 | Regeln `omvand-betalningsskyldighet` och momskoderna. | Uppfyllt när regeln är inlagd |
| Bara en människa bokför | Krävs inte av lag | Agenten kan bara föreslå. Varje bokföring signeras av en människa, med den säkerhetsnivå du valt. | Utöver lagen |
| Anläggningstillgångar skrivs av systematiskt över nyttjandeperioden | ÅRL 4 kap. 4 §, K2 punkt 10.23–10.27 | Inventarieregistret (`bok inventarie`) med avskrivning enligt plan, lika mycket varje månad från månaden inventariet tas i bruk. K2:s fem år godtas som nyttjandeperiod. Årets avskrivningar blir ett förslag (`bok bokslut avskrivningar`), och registret stäms av mot huvudboken. | Uppfyllt |
| Avyttring och utrangering | K2 punkt 7.13–7.14 | `bok inventarie avyttra`: avskrivning fram till avgången, och vinst eller förlust som övrig rörelseintäkt eller rörelsekostnad (3973, 7973). Momsen anges från fakturan. | Uppfyllt |
| Inventarier av mindre värde och korttidsinventarier | IL 18 kap. 4 §, IL 2 kap. 27 §, K2 punkt 10.5–10.6 | Anskaffningsvärde utan moms under ett halvt prisbasbelopp (29 600 kr 2026, förordning 2025:872) eller livslängd högst tre år: `bok inventarie lagg-till` säger att inventariet får dras av direkt. Prisbasbeloppet för året då beskattningsåret går ut. Regeln `inventarier` för agenten. | Uppfyllt; samband mellan inventarier bedömer du |
| Periodisering och beloppsgränsen | ÅRL 2 kap. 4 § första stycket 4, K2 punkt 2.4, 2.4B och 7.9 (BFNAR 2025:2) | `bok bokslut periodisera` räknar andelen per dag och föreslår periodiseringen och återföringen. Under 7 000 kr per faktura eller avtal (5 000 kr för år som började före 2026) föreslås den bara med `--anda`. | Uppfyllt; väsentlighet och återkommande utgifter (7.9) bedömer du |
| Bolagsskatt | IL 65 kap. 10 §, IL 1 kap. 7 § | 20,6 % av överskottet avrundat nedåt till helt tiotal kronor. `bok bokslut skatt` föreslår skatten (8910 mot 2512), eller skillnaden mot den bokförda. | Uppfyllt |
| Kostnader som inte får dras av och skattefria intäkter | IL 9 kap. 2, 4, 8 och 9 §§, 16 kap. 2 §, 8 kap. 7 § | Läggs tillbaka i INK2S 4.3a och 4.3c, och skattefri ränta dras av i 4.5c, enligt kontokopplingen [`ink2s-bas.txt`](ink2s-bas.txt). Regeln `bolagsskatt` för agenten. | Uppfyllt för konton i kopplingen |
| Inkomstdeklaration 2 som fil | Skatteverket, Teknisk information om filöverföring och fältförteckningarna för 2026P4 | `bok deklaration ink2` skriver `INFO.SRU` och `BLANKETTER.SRU` (ISO 8859-1, `INK2-2026P4`, `INK2R-2026P4`, `INK2S-2026P4`) och kontrollerar balans, resultat, fältkoder, datatyper och fält som utesluter varandra. Kontokopplingen följer BAS kopplingstabell ([`ink2r-bas.txt`](ink2r-bas.txt)). | Uppfyllt för 2026P4; uppgifter som inte syns i bokföringen anges för hand |
| Årsredovisning | BFL 6 kap., årsredovisningslagen | Bokis gör bokslutsposterna och deklarationen. Årsredovisningen (K2-dokumentet och iXBRL till Bolagsverket) ingår inte än. | Ingår inte |
| Personuppgifter i underlag | Dataskyddsförordningen (GDPR) | Allt lagras hos dig. En AI-agent i molnet ser de underlag den hämtar. | Ditt ansvar |

Förkortningar: BFL = bokföringslagen (1999:1078), BFNAR 2013:2 = BFN:s allmänna råd med
vägledning Bokföring, ML = mervärdesskattelagen (2023:200), IL = inkomstskattelagen (1999:1229),
ÅRL = årsredovisningslagen (1995:1554), K2 = BFNAR 2016:10 Årsredovisning i mindre företag, i
lydelsen efter BFNAR 2025:2.

## Grundregler för agenten

Agenten följer reglerna den hämtar med `hamta_regler` och anger dem i sina förslag. Grundreglerna
ligger i [`docs/regler/`](regler/), en fil per regel med lagrum och källor. De gäller först när du
har granskat dem och lagt in dem med din nyckel:

```bash
for f in docs/regler/*.json; do bok regel lagg-till "$(basename "$f" .json)" --file "$f"; done
```

| Regel | Innehåll |
|-------|----------|
| `verifikation` | Vad en verifikation ska visa, och när motpart får utelämnas |
| `momsavdrag` | Avdrag bara med faktura eller kvitto som visar momsen; förenklad faktura upp till 4 000 kr |
| `privata-kostnader` | Inget avdrag och ingen moms; agenten frågar vem som betalade |
| `representation` | Momsgränsen 300 kr per person, måltider ej avdragsgilla, enklare förtäring 60 kr, livsmedel 6 % från 1 april 2026 |
| `gavor` | Högst 300 kr exklusive moms per person, inte vid helger eller högtidsdagar |
| `omvand-betalningsskyldighet` | Konton och fält för inköp från EU, från länder utanför EU, byggtjänster och import |
| `inventarier` | Halvt prisbasbelopp och treårsregeln för direktavdrag, annars tillgång och inventarieregistret |
| `periodisering` | Interimskonton, K2:s gräns 7 000 kr per faktura och återkommande utgifter |
| `bolagsskatt` | 20,6 %, avrundning till tiotal kronor, och kostnader som läggs tillbaka |

## Ändrat vid kontrollen 2026-11-15

- Momsdeklarationens rubriker följde blanketten från före den nya mervärdesskattelagen, till
  exempel "skattskyldig" och "näringsidkare". De följer nu SKV 4700 utgåva 22.
- Momskoderna ([`momskoder-bas.txt`](momskoder-bas.txt)) är granskade mot BAS 2026 och
  Skatteverkets beskrivning av fälten. Egna uttag (fält 06) och frivilligt momspliktiga
  hyresintäkter (fält 08) är tillagda.
- Saknad motpart ger varning på alla förslag, inte bara på dem med underlag.
- Agentens instruktion säger vad en verifikation ska innehålla, när ingående moms får dras av och
  att privata kostnader inte hör till verksamheten.
- Grundreglerna ovan.

## Tillagt vid kontrollen 2026-11-16

- Inventarieregistret, avskrivning enligt plan, avyttring och förslag om årets avskrivningar.
- Periodisering med K2:s beloppsgräns. Gränsen höjdes från 5 000 till 7 000 kr genom BFNAR 2025:2,
  för räkenskapsår som börjar efter den 31 december 2025.
- Bolagsskatten och inkomstdeklaration 2 som SRU-filer, med fältförteckningen för 2026P4.
- Grundreglerna `inventarier`, `periodisering` och `bolagsskatt`.

## Kvar

- Systemdokumentationen behöver en beskrivning av arkiveringen (arkivplan).
- Motpartens adress kan inte sparas. BFN godtar namn och adress som identifiering, och för
  privatpersoner finns ofta inget organisationsnummer.
- Vinstmarginalbeskattning (fält 07) och trepartshandel (fält 37–38) har inga momskoder.
- Årsredovisningen (K2-dokumentet) och den digitala inlämningen till Bolagsverket.
- Fältförteckningen för INK2 finns bara för 2026P4. Ett räkenskapsår som slutar i en annan period
  kräver Skatteverkets förteckning för den perioden. BAS kopplingstabell anger "Konton i BAS 2023";
  stäm av konton som är nya i BAS 2026 mot din kontoplan.
- Lagen säger inget om avrundning av själva skatten; Bokis bokför skatten i öre, och Skatteverkets
  beslut om slutlig skatt gäller.

## Källor

- Bokföringslagen (1999:1078), ändrad t.o.m. SFS 2024:342:
  <https://www.riksdagen.se/sv/dokument-och-lagar/dokument/svensk-forfattningssamling/bokforingslag-19991078_sfs-1999-1078/>
- BFN, vägledning Bokföring (BFNAR 2013:2), uppdaterad 2024-09-16:
  <https://www.bfn.se/wp-content/uploads/vl13-2-bokforing.pdf>
- Mervärdesskattelagen (2023:200):
  <https://www.riksdagen.se/sv/dokument-och-lagar/dokument/svensk-forfattningssamling/mervardesskattelag-2023200_sfs-2023-200/>
- Inkomstskattelagen (1999:1229):
  <https://www.riksdagen.se/sv/dokument-och-lagar/dokument/svensk-forfattningssamling/inkomstskattelag-19991229_sfs-1999-1229/>
- Skatteverket, Fylla i momsdeklarationen:
  <https://www.skatteverket.se/foretag/moms/deklareramoms/fyllaimomsdeklarationen.4.3a2a542410ab40a421c80004214.html>
- Skatteverket, Momslagens regler om fakturering:
  <https://www.skatteverket.se/foretag/moms/saljavarorochtjanster/momslagensregleromfakturering.4.58d555751259e4d66168000403.html>
- Skatteverket, Köpa varor eller tjänster till företaget:
  <https://www.skatteverket.se/foretag/moms/kopavarorochtjanster/kopavarorellertjanstertillforetaget.4.7459477810df5bccdd480005156.html>
- Skatteverket, Avdrag för moms vid representation:
  <https://www.skatteverket.se/foretag/moms/kopavarorochtjanster/avdragformomsvidrepresentation.4.15532c7b1442f256baec84b.html>
- Skatteverket, Avdragslexikon:
  <https://www.skatteverket.se/foretag/skatterochavdrag/avdragforforetag/avdragslexikonforetag.4.3684199413c956649b550c8.html>
- Skatteverket, Omvänd betalningsskyldighet:
  <https://www.skatteverket.se/foretag/moms/sarskildamomsregler/omvandbetalningsskyldighet.4.47eb30f51122b1aaad28000258292.html>
- BAS 2026: <https://www.bas.se/kontoplaner/>
- Årsredovisningslagen (1995:1554):
  <https://www.riksdagen.se/sv/dokument-och-lagar/dokument/svensk-forfattningssamling/arsredovisningslag-19951554_sfs-1995-1554/>
- Förordning (2025:872) om prisbasbelopp för 2026: <https://data.riksdagen.se/dokument/sfs-2025-872.text>
- BFN, vägledning Årsredovisning i mindre företag (K2, BFNAR 2016:10), uppdaterad 2026-10-29:
  <https://www.bfn.se/wp-content/uploads/vl16-10-k2-kons2026.pdf>
- Skatteverket, Belopp och procent 2026:
  <https://skatteverket.se/foretag/skatterochavdrag/beloppochprocent/2026.106.1522bf3f19aea8075ba3294.html>
- Skatteverket, Skatteregler för aktie- och handelsbolag (SKV 294 utg. 24):
  <https://www.skatteverket.se/download/18.6491ed82199c4722daef0c/1761209691103/skatteregler-for-aktie-och-handelsbolag-skv294-utgava24.pdf>
- Skatteverket, Teknisk information om filöverföring (SRU-formatet och fältförteckningarna för 2026P4):
  <https://skatteverket.se/foretag/inkomstdeklaration/forredovisningsbyraer/tekniskinformationomfiloverforing.4.13948c0e18e810bfa0cca8.html>
- BAS, kopplingstabell BAS–INK2: <https://www.bas.se/kontoplaner/sru/>
