#!/usr/bin/env bash
# Provar punkterna ur checklistan från 2026-11-17 i en ny provbokföring med en
# engångsnyckel (nivå mjukvara) och ett påhittat bolag: kontrollen av
# organisationsnummer, motpartens adress (schemaversion 5), arkivplanen i
# systemdokumentationen, insyn per person med en riktig insynstjänst och
# loggade inloggningar, vinstmarginal och trepartshandel i momsrapporten och
# MCP 2026-07-28 (server/discover) mot bok-mcp. Slutar med en full verifiering.
#
#   cargo build -p bok-cli -p bok-mcp
#   scripts/e2e-checklista.sh /tmp/bokis-checklista     # katalogen får inte finnas
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

fail() {
    echo "FEL: $*" >&2
    exit 1
}

echo "== 1. Organisationsnumret kontrolleras innan något sparas"
if "$BOK" init --security mjukvara --company "Provbolaget AB" --orgnr 559000-0000 --year 2026 2>"$WORK/fel.txt"; then
    fail "559000-0000 gick igenom"
fi
tail -1 "$WORK/fel.txt"
[ ! -e "$BOKIS_DB" ] || fail "en misslyckad init lämnade en databas"
"$BOK" init --security mjukvara --company "Provbolaget AB" --orgnr 5590000005 --year 2026
cat > "$WORK/konton.txt" <<'EOF'
1930;Företagskonto
2081;Aktiekapital
2611;Utgående moms 25 %;10
3021;Försäljning, vinstmarginalbeskattning;07
4021;Inköp, vinstmarginalbeskattning;07
3046;Försäljning, trepartshandel;38
4046;Inköp, trepartshandel;37
5410;Förbrukningsinventarier
EOF
"$BOK" konto importera "$WORK/konton.txt" | tail -1

echo "== 2. Motpartens nummer och adress"
if "$BOK" foresla --date 2026-03-02 --text "Begagnad cykel" --line 4021=6000 --line 1930=-6000 \
    --counterparty-name "Cykelhandlarn AB" --orgnr 556000-0000 2>"$WORK/fel.txt"; then
    fail "fel kontrollsiffra hos motparten gick igenom"
fi
tail -1 "$WORK/fel.txt"
"$BOK" foresla --date 2026-03-02 --text "Inköp av begagnad cykel från privatperson" \
    --line 4021=6000 --line 1930=-6000 --counterparty-name "Kim Exempel" \
    --motpart-adress $'Exempelgatan 1\n123 45 Exempelstad' --approve | tail -1
"$BOK" visa A1 > "$WORK/a1.txt"
grep "Motpart" "$WORK/a1.txt" || fail "motparten syns inte"
grep -q "Exempelgatan 1, 123 45 Exempelstad" "$WORK/a1.txt" || fail "adressen sparades inte på en rad"
"$BOK" motpart lagg-till --name "Kunden Oy" --orgnr "fi 1234567-8" --adress "Esimerkkikatu 1, Helsingfors"
"$BOK" motpart lista

echo "== 3. Moms: vinstmarginal (ruta 07) och trepartshandel (37–38)"
"$BOK" foresla --date 2026-03-09 --text "Försäljning av begagnad cykel" --line 1930=10000 \
    --line 3021=-9200 --line 2611=-800 --counterparty-name "Kim Köpare" \
    --motpart-adress "Köpargatan 2, 123 46 Exempelstad" --approve | tail -1
"$BOK" foresla --date 2026-03-10 --text "Inköp från tysk leverantör, levererat till kund i Finland" \
    --line 4046=5000 --line 1930=-5000 --counterparty-name "Lieferant GmbH" --orgnr DE123456789 --approve | tail -1
"$BOK" foresla --date 2026-03-11 --text "Försäljning till finsk kund, trepartshandel" \
    --line 1930=6000 --line 3046=-6000 --counterparty-name "Kunden Oy" --orgnr FI12345678 --approve | tail -1
"$BOK" moms 2026-03 | tee "$WORK/moms.txt"
grep -q "Ruta 07.* 3 200,00" "$WORK/moms.txt" || fail "ruta 07 ska vara 3 200,00"
grep -q "periodiska sammanställningen" "$WORK/moms.txt" || fail "påminnelsen om ruta 38 saknas"

echo "== 4. Arkivplanen"
"$BOK" arkivplan visa > "$WORK/arkivplan-fore.txt"
head -3 "$WORK/arkivplan-fore.txt"
sed "s|\"/mnt/kopior\"|\"$WORK/kopior\"|" "$ROOT/docs/arkivplan-exempel.json" > "$WORK/arkivplan.json"
mkdir -p "$WORK/kopior"
"$BOK" kopia skapa "$WORK/kopior" > "$WORK/kopia.txt"
head -1 "$WORK/kopia.txt"
"$BOK" arkivplan satt "$WORK/arkivplan.json"
"$BOK" arkivplan visa > "$WORK/arkivplan.txt"
head -12 "$WORK/arkivplan.txt"
grep -q "bevaras till och med 2033-12-31" "$WORK/arkivplan.txt" || fail "bevarandetiden saknas"
"$BOK" systemdokumentation "$WORK/systemdokumentation.md" | tail -1
grep -q "^## Arkivplan" "$WORK/systemdokumentation.md" || fail "arkivplanen saknas i systemdokumentationen"

echo "== 5. Insyn per person, med en riktig insynstjänst"
PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
ORIGIN="https://bokis.example.ts.net:8443"
"$BOK" insyn --port "$PORT" --adress "$ORIGIN" > "$WORK/insyn.log" 2>&1 &
INSYN=$!
trap 'kill $INSYN 2>/dev/null || true' EXIT
for _ in $(seq 50); do
    grep -q "nås på" "$WORK/insyn.log" && break
    sleep 0.1
done
"$BOK" insyn lank --namn "Revisor X" --giltig 30d | tee "$WORK/lank.txt"
URL=$(grep -o "$ORIGIN/login?token=[0-9a-f]*" "$WORK/lank.txt")
ID=$(sed -n 's/^Insynslänk \([0-9]*\) .*/\1/p' "$WORK/lank.txt")
LOCAL=${URL/$ORIGIN/http://127.0.0.1:$PORT}
HOST="bokis.example.ts.net:8443"
curl -s -o /dev/null -D "$WORK/huvud.txt" -H "Host: $HOST" "$LOCAL"
grep -i "^set-cookie" "$WORK/huvud.txt"
COOKIE=$(sed -n 's/^[Ss]et-[Cc]ookie: \([^;]*\);.*/\1/p' "$WORK/huvud.txt" | tr -d '\r')
[ -n "$COOKIE" ] || fail "ingen sessionskaka"
STATUS=$(curl -s -o /dev/null -w '%{http_code}' -H "Host: $HOST" -H "Cookie: $COOKIE" "http://127.0.0.1:$PORT/")
echo "sidan med kakan: $STATUS"
[ "$STATUS" = 200 ] || fail "kakan släpper inte in"
"$BOK" insyn lankar
"$BOK" historik > "$WORK/historik.txt"
grep "insyn.inloggning" "$WORK/historik.txt" || fail "inloggningen loggades inte"
"$BOK" insyn aterkalla "$ID"
STATUS=$(curl -s -o /dev/null -w '%{http_code}' -H "Host: $HOST" -H "Cookie: $COOKIE" "http://127.0.0.1:$PORT/")
echo "sidan efter återkallelsen: $STATUS"
[ "$STATUS" = 403 ] || fail "en återkallad session släpps in"
kill $INSYN
trap - EXIT

echo "== 6. MCP 2026-07-28 mot bok-mcp: server/discover och anrop utan initialize"
python3 -I - "$PARTS/bok-mcp" "$BOKIS_DB" <<'EOF'
import json, subprocess, sys

mcp, db = sys.argv[1], sys.argv[2]
meta = {"io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "e2e", "version": "1"},
        "io.modelcontextprotocol/clientCapabilities": {}}
msgs = [{"jsonrpc": "2.0", "id": 1, "method": "server/discover", "params": {"_meta": meta}},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {"_meta": meta}},
        {"jsonrpc": "2.0", "id": 3, "method": "tools/call",
         "params": {"name": "hamta_kontoplan", "arguments": {}, "_meta": meta}}]
out = subprocess.run([mcp, "--db", db], input="\n".join(json.dumps(m) for m in msgs) + "\n",
                     capture_output=True, text=True, check=True).stdout
replies = [json.loads(line) for line in out.splitlines()]
d = replies[0]["result"]
assert d["supportedVersions"] == ["2026-07-28"], d
assert all(r["result"]["resultType"] == "complete" for r in replies), replies
print("  server/discover:", d["supportedVersions"], d["_meta"]["io.modelcontextprotocol/serverInfo"])
print("  tools/list:", len(replies[1]["result"]["tools"]), "verktyg")
print("  tools/call hamta_kontoplan: ok")
EOF

echo "== 7. Insynen som export och verifiering"
"$BOK" insyn export "$WORK/export" | tail -1
grep -q "Exempelgatan 1, 123 45 Exempelstad" "$WORK/export/ar-2026/v-A1.html" || fail "adressen saknas i insynen"
grep -q "Arkivplan" "$WORK/export/systemdokumentation.html" || fail "arkivplanen saknas i insynen"
"$BOK" verifiera | tail -3
echo "Klart: alla steg gick igenom."
