# Agentmätning 2026-11-14: lokal Qwen och Claude mot bokföringssviten

Svar på arkitekturens öppna fråga 4: "Räcker en lokal modell för kontering?
Utvärdera Qwen mot testsviten innan en molnmodell övervägs."

**Kort svar: den lokala modellen räcker inte, men Claude gör det.** Med samma verktyg, samma
fråga och samma uppgifter:

| | Qwen3.6-35B (lokal) | Claude Opus 5.5 (moln) |
| --- | --- | --- |
| Besvarade | 49 av 52 | 52 av 52 |
| Godkända (konto och moms) | 1 (2 %) | 44 (85 %) |
| Godtaget konto | 1 (2 %) | 51 (98 %), varav 38 exakt facitkontot |
| Rätt momsbehandling | 35 (67 %) | 44 (85 %) |
| Tid per uppgift | 5 min 12 s | 16 s |

Claudes alla åtta underkända gäller uppgifter där det bara finns en banktransaktion och
inget kvitto. Där drog Claude medvetet ingen moms, med motiveringen att avdrag för ingående
moms kräver en faktura eller ett kvitto som visar momsen. Den föreslog att momsen flyttas
till 2641 när kvittot kommer. Det stämmer med mervärdesskattelagens krav på underlag, men
benchens facit förutsätter momskategorin ändå. Räknar man de åtta som försvarbara är Claude
rätt i 51 av 52.

Claude kördes som `claude -p` (Claude Code 2.1.314) med bara `bok-eval`-servern som verktyg,
utan Henriks inställningar och `CLAUDE.md`, på hans prenumeration. Uppgifterna, som är publika
benchdata, skickades alltså till Anthropic. Gränsen för MCP-svar höjdes
(`MAX_MCP_OUTPUT_TOKENS=100000`), så att Claude fick hela kontoplanen precis som Qwen.

## Hur mätningen gjordes

- **Modell:** Qwen3.6-35B-A3B (Q8_0) i llama.cpp på den här datorn, via Qwen Code 0.26.6.
  Inget lämnade datorn.
- **Uppgifter:** de 52 frysta bokföringsuppgifterna i Accounted Bench
  (uppgifter och facit © 2025–2026 Jakob Wennberg, CC BY 4.0,
  <https://github.com/erp-mafia/accounted-bench>, fryst 2026-09-01).
- **Arbetssätt:** som i riktig användning, genom `bok-eval serve`, alltså MCP-servern.
  - En agentsession per uppgift.
  - Agenten läste underlaget, reglerna och benchens kontoplan (1 290 BAS-konton) och lade ett
    verifikationsförslag med hela konteringen.
  - Samma frågetext för alla uppgifter, utan exempel. Modellens tänkande var påslaget.
- **Bedömning:** `bok-eval score`.
  - Kontot är det som bär störst belopp efter att bank-, reskontra- och momskonton räknats
    bort.
  - Momsbehandlingen härleds ur momskontona och den avdragna momsens andel.
  - Siffrorna är därför inte direkt jämförbara med benchens topplista, där modellen svarar
    med ett konto och en momskod.

## Qwen i detalj

| | |
| --- | --- |
| Besvarade | 49 av 52 (3 avbröts efter 15 minuter utan förslag) |
| Godkända (konto och moms) | 1 (2 %) |
| Godtaget konto | 1 (2 %) |
| Rätt momsbehandling | 35 (67 %) |
| Tid per uppgift | i snitt 5 min 12 s |

Momsbehandlingen per kategori, besvarade uppgifter:

| Facit | Rätt | Fel |
| --- | --- | --- |
| Svensk moms 25 % | 15 | 3 |
| Svensk moms 12 % | 2 | 0 |
| Svensk moms 6 % | 4 | 2 |
| Ingen moms | 7 | 3 |
| Omvänd betalningsskyldighet, tjänster | 5 | 2 |
| Omvänd betalningsskyldighet, bygg | 1 | 0 |
| Import | 1 | 0 |
| Omvänd betalningsskyldighet, varor | 0 | 1 |
| Representation med beloppstak | 0 | 2 |
| Halvt avdrag (leasingbil) | 0 | 1 |

## Vad som gick fel

Modellen hämtade kontoplanen och reglerna, ungefär två gånger per uppgift, men valde ändå
konton som finns i BAS och inte passar händelsen. Servern nekar konton som inte finns, så alla
är riktiga BAS-konton.

| Uppgift | Konto enligt facit | Modellens konto |
| --- | --- | --- |
| Redovisningstjänster | 6530 | 2638, vilande utgående moms |
| Mobiltelefon | 6212 | 6490 |
| Kontorsmateriel | 6110 | 6490 |
| Försäkringspremie | 6310 | 3910, med 25 % moms |
| El | 5020 | 5110 |

Många förslag krävde flera försök innan debet och kredit balanserade.

## Claude i detalj

Momsbehandlingen per kategori, alla 52 besvarade:

| Facit | Rätt | Fel |
| --- | --- | --- |
| Svensk moms 25 % | 15 | 4 |
| Svensk moms 12 % | 2 | 1 |
| Svensk moms 6 % | 4 | 3 |
| Ingen moms | 10 | 0 |
| Omvänd betalningsskyldighet, tjänster | 7 | 0 |
| Omvänd betalningsskyldighet, bygg | 1 | 0 |
| Omvänd betalningsskyldighet, varor | 1 | 0 |
| Import | 1 | 0 |
| Representation med beloppstak | 2 | 0 |
| Halvt avdrag (leasingbil) | 1 | 0 |

Alla fel på svensk moms är uppgifter utan kvitto, se ovan. Det enda kontofelet var
friskvård (SATS), där Claude valde 7620 i stället för 7699 eller 7690.

## Slutsats och nästa steg

1. **En molnmodell klarar konteringen.** Claude Opus 5.5 valde godtaget konto i 51 av 52
   uppgifter och hanterade alla specialfall rätt: omvänd betalningsskyldighet, import,
   representation och halvt avdrag.
2. **Den lokala modellen gör det inte än.** Kontot är det svåra för den. Billigare hjälp
   värd att pröva innan den avskrivs helt:
   - en kontoplan med bolagets egna konton i stället för alla 1 290
   - regler per leverantör (`bok regel`)
   - tidigare konteringar för samma motpart (`sok_motpart` hittar dem)
3. **Frågan blir då var underlagen får skickas.** En molnmodell innebär att kvitton och
   fakturor lämnar datorn och går till Anthropic. Det är en fråga om förvaring och
   personuppgifter, som regelkontrollen pekade ut, inte om förmåga.
4. **Mät gärna Sonnet 5.5 på samma sätt.** Den är snabbare och billigare, och skillnaden mot
   Opus avgör vilken som behövs.

Råmaterialet ligger utanför repot, i `~/bokis-ai-first-delar/agentmatning-2026-11-14/`
(Qwen i katalogen själv, Claude i `claude-opus-5-5/`): `forslag.jsonl`, `resultat.json`,
`tider.txt`, körskriptet och agentens utdata per uppgift. Det
innehåller benchens uppgifter och facit och hör inte hemma i ett publikt repo, eftersom benchen
ber att uppgifterna hålls borta från träningsdata.
