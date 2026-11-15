# Lagkontroll: vilka lagar och krav Bokis uppfyller

Kontrollerad 2026-11-15 mot lagtexterna på riksdagen.se, BFN:s vägledning Bokföring och
Skatteverkets sidor. Lagar och belopp ändras: kontrollera igen minst en gång om året och när
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
| Årsbokslut och årsredovisning | BFL 6 kap., årsredovisningslagen | Bokis gör bokslutsposterna (årets resultat och ingående balans). Årsredovisningen ingår inte. | Ingår inte |
| Personuppgifter i underlag | Dataskyddsförordningen (GDPR) | Allt lagras hos dig. En AI-agent i molnet ser de underlag den hämtar. | Ditt ansvar |

Förkortningar: BFL = bokföringslagen (1999:1078), BFNAR 2013:2 = BFN:s allmänna råd med
vägledning Bokföring, ML = mervärdesskattelagen (2023:200), IL = inkomstskattelagen (1999:1229).

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

## Kvar

- Systemdokumentationen behöver en beskrivning av arkiveringen (arkivplan).
- Motpartens adress kan inte sparas. BFN godtar namn och adress som identifiering, och för
  privatpersoner finns ofta inget organisationsnummer.
- Vinstmarginalbeskattning (fält 07) och trepartshandel (fält 37–38) har inga momskoder.

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
