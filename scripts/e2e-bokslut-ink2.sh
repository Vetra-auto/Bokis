#!/usr/bin/env bash
# Provar bokslutet och inkomstdeklarationen från början till slut i en ny
# provbokföring med en engångsnyckel (nivå mjukvara) och ett påhittat bolag:
# verifikationer, inventarieregistret, agentens avskrivningsförslag genom
# bok-mcp, periodisering med återföring, bolagsskatt, årets resultat,
# INK2 som SRU-filer, insynen som export och en full verifiering.
#
#   cargo build -p bok-cli -p bok-mcp
#   scripts/e2e-bokslut-ink2.sh /tmp/bokis-bokslut     # katalogen får inte finnas
#
# BOK och PARTS pekar på binärerna (standard: target/debug i arbetsytan).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
BOK=${BOK:-$ROOT/target/debug/bok}
PARTS=${PARTS:-$ROOT/target/debug}
WORK=${1:?ange en katalog som inte finns}
if [ -e "$WORK" ]; then
    echo "$WORK finns redan" >&2
    exit 1
fi
mkdir -p "$WORK/bok"
ssh-keygen -q -t ed25519 -N '' -C bokis-e2e -f "$WORK/nyckel"
export BOKIS_DB="$WORK/bok/bokis.db" BOKIS_KEY="$WORK/nyckel" BOKIS_USER=granskare

agent() {
    python3 -I - "$PARTS/bok-mcp" "$BOKIS_DB" "$1" <<'EOF'
import json, subprocess, sys

mcp, db, calls = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
msgs = [{"jsonrpc": "2.0", "id": 0, "method": "initialize",
         "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": {"name": "e2e", "version": "1"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"}]
msgs += [{"jsonrpc": "2.0", "id": i + 1, "method": "tools/call",
          "params": {"name": n, "arguments": a}} for i, (n, a) in enumerate(calls)]
out = subprocess.run([mcp, "--db", db], input="\n".join(json.dumps(m) for m in msgs) + "\n",
                     capture_output=True, text=True, check=True).stdout
for line in out.splitlines():
    r = json.loads(line)
    if r["id"] == 0:
        continue
    name = calls[r["id"] - 1][0]
    text = r["result"]["content"][0].get("text", "")
    first = " / ".join(text.splitlines()[:3]) if text else ""
    print(f"  {name}: {'fel' if r['result'].get('isError') else 'ok'}: {first[:200]}")
EOF
}
HARKOMST='{"modell": "e2e-skript", "region": "lokal", "motivering": "Årets avskrivningar enligt registret.", "osakerhet": 0.1}'

# Godkänner det senaste förslaget.
approve_last() {
    local id
    id=$("$BOK" forslag | awk '/^Förslag [0-9]+/ {print $2}' | tail -1)
    "$BOK" godkann "$id" | tail -1
}

echo "== 1. Provbolaget, första räkenskapsåret 2026-04-27 – 2026-12-31, och kontoplanen"
"$BOK" init --security mjukvara --company "Provbolaget AB" --orgnr 559000-0005 --year 2026-04-27:2026-12-31
"$BOK" ar lagg-till 2027
cat > "$WORK/konton.txt" <<'EOF'
1220;Inventarier och verktyg
1229;Ackumulerade avskrivningar på inventarier och verktyg
1630;Avräkning för skatter och avgifter (skattekonto)
1790;Övriga förutbetalda kostnader och upplupna intäkter
1930;Företagskonto
2081;Aktiekapital
2099;Årets resultat
2512;Beräknad inkomstskatt
2611;Utgående moms på försäljning inom Sverige, 25 %
2641;Debiterad ingående moms
2990;Övriga upplupna kostnader och förutbetalda intäkter
3001;Försäljning inom Sverige, 25 % moms
5420;Programvaror
6072;Representation, ej avdragsgill
6310;Företagsförsäkringar
7010;Löner till kollektivanställda
7832;Avskrivningar på inventarier och verktyg
8314;Skattefria ränteintäkter
8910;Skatt som belastar årets resultat
8999;Årets resultat
EOF
"$BOK" konto importera "$WORK/konton.txt"

echo "== 2. Årets verifikationer"
"$BOK" foresla --date 2026-04-27 --text "Aktiekapital" --line 1930=25000 --line 2081=-25000 --approve | tail -1
"$BOK" foresla --date 2026-05-12 --text "Bärbar dator, faktura 4711" --line 1220=36000 --line 2641=9000 \
    --line 1930=-45000 --counterparty-name "Datorbutiken AB" --orgnr 556000-0100 --approve | tail -1
"$BOK" foresla --date 2026-09-01 --text "Konsultuppdrag" --line 1930=250000 --line 3001=-200000 \
    --line 2611=-50000 --counterparty-name "Kunden AB" --orgnr 556000-0100 --approve | tail -1
"$BOK" foresla --date 2026-09-15 --text "Programvara" --line 5420=30000 --line 2641=7500 --line 1930=-37500 \
    --counterparty-name "Programhuset AB" --orgnr 556000-0100 --approve | tail -1
"$BOK" foresla --date 2026-10-02 --text "Kundmiddag" --line 6072=2000 --line 1930=-2000 \
    --counterparty-name "Restaurangen AB" --orgnr 556000-0100 --approve | tail -1
"$BOK" foresla --date 2026-11-01 --text "Företagsförsäkring nov 2026 – okt 2027" --line 6310=12000 \
    --line 1930=-12000 --counterparty-name "Försäkringsbolaget AB" --orgnr 556000-0100 --approve | tail -1
"$BOK" foresla --date 2026-11-25 --text "Lön november" --line 7010=20000 --line 1930=-20000 --approve | tail -1
"$BOK" foresla --date 2026-12-01 --text "Intäktsränta skattekontot" --line 1630=100 --line 8314=-100 --approve | tail -1

echo "== 3. Inventarieregistret"
"$BOK" inventarie lagg-till --name "Bärbar dator" --date 2026-05-12 --amount 36000 --life 36 --voucher A2
echo "-- en skärm under halva prisbasbeloppet registreras av misstag och stryks:"
"$BOK" inventarie lagg-till --name "Skärm" --date 2026-05-12 --amount 4000 --life 60
"$BOK" inventarie avyttra 2 --date 2026-05-12 --kind felregistrering --note "Förbrukningsinventarie, kostnadsförs direkt"
"$BOK" inventarie plan 1
"$BOK" inventarie lista --year 2026

echo "== 4. Agenten ser registret och föreslår årets avskrivningar genom bok-mcp"
agent "[[\"lista_inventarier\", {}], [\"foresla_avskrivningar\", {\"ar\": \"2026\", \"harkomst\": $HARKOMST}]]"
approve_last
agent "[[\"foresla_avskrivningar\", {\"ar\": \"2026\", \"harkomst\": $HARKOMST}]]"

echo "== 5. Periodisering av försäkringen, med återföring 2027-01-01"
"$BOK" bokslut periodisera --year 2026 --kind forutbetald-kostnad --account 6310 --amount 12000 \
    --from 2026-11-01 --to 2027-10-31 --text "Företagsförsäkring" --aterfor
"$BOK" godkann "$("$BOK" forslag | awk '/^Förslag [0-9]+/ {print $2}' | head -1)" | tail -1
echo "-- en faktura under 7 000 kr periodiseras inte utan --anda:"
"$BOK" bokslut periodisera --year 2026 --kind forutbetald-kostnad --account 5420 --amount 6000 \
    --from 2026-12-01 --to 2027-05-31 --text "Licens" 2>&1 | tail -1 || true

echo "== 6. Bolagsskatt och årets resultat"
"$BOK" bokslut skatt --year 2026
approve_last
"$BOK" bokslut skatt --year 2026
"$BOK" bokslut resultat --year 2026 | tail -2
approve_last

echo "== 7. INK2 som SRU-filer"
"$BOK" deklaration ink2 --year 2026 "$WORK/ink2" --postnr "123 45" --postort Småstad \
    --kontakt "Kim Exempel" --falt 8041=X --falt 8045=X
file "$WORK/ink2/INFO.SRU" "$WORK/ink2/BLANKETTER.SRU"
echo "-- INFO.SRU:"
iconv -f ISO-8859-1 -t UTF-8 "$WORK/ink2/INFO.SRU"
echo "-- BLANKETTER.SRU:"
iconv -f ISO-8859-1 -t UTF-8 "$WORK/ink2/BLANKETTER.SRU"
echo "-- befintliga filer skrivs inte över:"
"$BOK" deklaration ink2 --year 2026 "$WORK/ink2" --postnr 12345 --postort Småstad 2>&1 | tail -1 || true

echo "== 8. Insynen som export, och verifiering"
"$BOK" insyn export "$WORK/insyn" --year 2026 | tail -1
grep -o "Bolagsskatt</h2>[^<]*<p>[^<]*<strong>[^<]*" "$WORK/insyn/ar-2026/deklaration.html" || true
"$BOK" verifiera | tail -3
