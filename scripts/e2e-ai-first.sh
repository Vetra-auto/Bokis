#!/bin/bash
# Hela flödet en gång, från tom bokföring till stängd period och inläst kopia:
#
#   1. bok skapar bokföringen; fakturan läses in från inkorgen
#   2. agenten (här ett skript) läser fakturan, föreslår och frågar genom bok-mcp
#   3. granskaren godkänner i bok-review (ssh-keygen signerar) och svarar på frågan
#   4. kontoutdraget läses in; agenten ser den omatchade bankraden och föreslår
#      betalningen; granskaren godkänner och stämmer av raden
#   5. momsrapporten tas fram och perioden stängs, vilket bara går när varje
#      bankrad är avstämd
#   6. bok visar grundboken, verifierar kedja, signaturer, arkiv och bank och
#      skriver systemdokumentationen
#   7. bok skriver en kopia, kontrollerar den och läser in den i en tom katalog
#   8. insynen exporteras för revisorn och kontrolleras med sha256sum, utan Bokis
#   9. bok-fjarr: en agent ansluter över HTTP med kopplingskod och PKCE, telefonen
#      hämtar sin länk med en kod på startsidan och skickar ett kvitto, och
#      agenten hämtar det
#
# Allt skapas under en ny katalog, med en engångsnyckel utan lösenord.
# Inget annat rörs.
#
#   cargo build -p bok-cli -p bok-mcp -p bok-review -p bok-fjarr
#   scripts/e2e-ai-first.sh /tmp/bokis-e2e
#
# BOK och PARTS pekar på binärerna (standard: target/debug i arbetsytan).

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
BOK=${BOK:-$ROOT/target/debug/bok}
PARTS=${PARTS:-$ROOT/target/debug}
WORK=${1:?ange en katalog som inte finns}
if [ -e "$WORK" ]; then
    echo "$WORK finns redan" >&2
    exit 2
fi

mkdir -p "$WORK/bok" "$WORK/inkorg"
ssh-keygen -q -t ed25519 -N '' -C bokis-e2e -f "$WORK/nyckel"
export BOKIS_DB="$WORK/bok/bokis.db" BOKIS_KEY="$WORK/nyckel" BOKIS_USER=granskare
YEAR=$(date +%Y)
MONTH=$(date +%Y-%m)
TODAY=$(date +%F)

# Agenten: anrop till bok-mcp över stdio. Argument: en JSON-lista med [verktyg, argument].
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
    if "error" in r:
        print(f"  {name}: nekat ({r['error']['code']}) {r['error']['message']}")
    else:
        text = r["result"]["content"][0].get("text", "")
        first = " / ".join(text.splitlines()[:2]) if text else ""
        print(f"  {name}: {'fel' if r['result'].get('isError') else 'ok'}: {first[:150]}")
EOF
}

HARKOMST='{"modell": "e2e-skript", "region": "lokal", "motivering": "Enligt underlaget.", "osakerhet": 0.1}'

echo "== 1. Bokföringen skapas och fakturan läses in från inkorgen"
"$BOK" init --security mjukvara --company "Testbolaget AB" --orgnr 559999-9999 --year "$YEAR"
printf '1930;Företagskonto\n2440;Leverantörsskulder\n2641;Debiterad ingående moms\n2650;Redovisningskonto för moms\n5420;Programvaror\n6540;IT-tjänster\n' \
    > "$WORK/konton.txt"
"$BOK" account import "$WORK/konton.txt" --vat-codes "$ROOT/docs/momskoder-bas.txt"
printf 'Exempel Hosting AB\norg.nr 556000-0000\nFaktura 2026-311\nMolntjänst 400,00 kr\nMoms 25 %% 100,00 kr\nAtt betala 500,00 kr\n' \
    > "$WORK/inkorg/faktura-2026-311.txt"
"$BOK" inkorg "$WORK/inkorg" --min-age 0

echo "== 2. Agenten läser fakturan och föreslår genom bok-mcp"
agent "[[\"lista_underlag\", {}],
        [\"hamta_underlag\", {\"id\": \"1\"}],
        [\"foresla_verifikation\", {\"affarshandelse_datum\": \"$TODAY\",
          \"beskrivning\": \"Molntjänst, faktura 2026-311 från Exempel Hosting AB\",
          \"motpart\": {\"namn\": \"Exempel Hosting AB\", \"orgnr\": \"556000-0000\"},
          \"rader\": [{\"konto\": 5420, \"debet_ore\": 40000}, {\"konto\": 2641, \"debet_ore\": 10000},
                      {\"konto\": 2440, \"kredit_ore\": 50000}],
          \"underlag_ids\": [\"1\"], \"harkomst\": $HARKOMST}],
        [\"stall_fraga\", {\"underlag_id\": \"1\", \"fraga\": \"Gäller fakturan hela november?\",
          \"harkomst\": $HARKOMST}],
        [\"bokfor_forslag\", {\"forslag_id\": \"1\"}]]"
"$BOK" proposals

echo "== 3. Granskaren godkänner i bok-review"
coproc REVIEW { exec "$PARTS/bok-review" --db "$BOKIS_DB" --nyckel "$BOKIS_KEY" --granskare granskare --port 0 2>"$WORK/review.err"; }
RPID=$REVIEW_PID
trap 'kill "${RPID:-}" 2>/dev/null || true' EXIT
read -r _ <&"${REVIEW[0]}"
read -r URL <&"${REVIEW[0]}"
URL=${URL// /}
BASE=${URL%%/login*}
JAR="$WORK/cookies"
curl -s -c "$JAR" -o /dev/null "$URL"
approve_first() {
    local id csrf
    id=$(curl -s -b "$JAR" "$BASE/" | grep -o 'href="/forslag/[0-9]*"' | head -1 | grep -o '[0-9]*')
    csrf=$(curl -s -b "$JAR" "$BASE/forslag/$id" | grep -o 'name="csrf" value="[0-9a-f]*"' | head -1 | sed 's/.*value="//; s/"$//')
    curl -s -b "$JAR" -H "Origin: $BASE" --data "csrf=$csrf" "$BASE/forslag/$id/godkann" \
        | grep -o '<h1>[^<]*</h1><p>[^<]*</p>' | sed 's#</h1><p># – #; s#<[^>]*>##g'
}
approve_first
"$BOK" arende lista
"$BOK" arende svara 1 "Ja, hela november."

echo "== 4. Kontoutdraget, agentens betalningsförslag och avstämningen"
cat > "$WORK/utdrag.xml" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02"><BkToCstmrStmt><Stmt><Id>1</Id>
 <Acct><Id><IBAN>SE4550000000058398257466</IBAN></Id><Ccy>SEK</Ccy></Acct>
 <Bal><Tp><CdOrPrtry><Cd>OPBD</Cd></CdOrPrtry></Tp><Amt Ccy="SEK">1000.00</Amt><CdtDbtInd>CRDT</CdtDbtInd><Dt><Dt>$TODAY</Dt></Dt></Bal>
 <Bal><Tp><CdOrPrtry><Cd>CLBD</Cd></CdOrPrtry></Tp><Amt Ccy="SEK">500.00</Amt><CdtDbtInd>CRDT</CdtDbtInd><Dt><Dt>$TODAY</Dt></Dt></Bal>
 <Ntry><Amt Ccy="SEK">500.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts>BOOK</Sts>
  <BookgDt><Dt>$TODAY</Dt></BookgDt><ValDt><Dt>$TODAY</Dt></ValDt><AcctSvcrRef>E2E-1</AcctSvcrRef>
  <NtryDtls><TxDtls><RmtInf><Ustrd>Exempel Hosting 2026311</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
</Stmt></BkToCstmrStmt></Document>
EOF
"$BOK" bank importera "$WORK/utdrag.xml" | tail -3
agent "[[\"lista_omatchade_bankrader\", {}],
        [\"foresla_verifikation\", {\"affarshandelse_datum\": \"$TODAY\",
          \"beskrivning\": \"Betalning av faktura 2026-311 till Exempel Hosting AB\",
          \"motpart\": {\"namn\": \"Exempel Hosting AB\"},
          \"rader\": [{\"konto\": 2440, \"debet_ore\": 50000}, {\"konto\": 1930, \"kredit_ore\": 50000}],
          \"underlag_ids\": [\"2\"], \"bankrad_ids\": [\"1\"], \"harkomst\": $HARKOMST}]]"
approve_first
"$BOK" bank forslag
"$BOK" bank matcha 1 A2
kill "$RPID"

echo "== 5. Momsrapporten och periodstängningen (kräver att varje bankrad är avstämd)"
"$BOK" moms "$MONTH"
"$BOK" period review "$MONTH"
"$BOK" period close "$MONTH"

echo "== 6. Grundboken, verifieringen och systemdokumentationen"
"$BOK" journal
"$BOK" verify
"$BOK" systemdokumentation "$WORK/systemdokumentation.md"

echo "== 7. Kopia, kontroll och läs-in-övning"
"$BOK" kopia skapa "$WORK/usb"
KOPIA=$("$BOK" kopia lista "$WORK/usb" | tail -1)
"$BOK" kopia kontrollera "$KOPIA"
"$BOK" kopia las-in "$KOPIA" "$WORK/provlasning"

echo "== 8. Insynen för revisor och Skatteverket, kontrollerad utan Bokis"
"$BOK" insyn export "$WORK/insyn"
(cd "$WORK/insyn" && sha256sum -c --quiet manifest.sha256 && echo "  manifestet stämmer för $(wc -l < manifest.sha256) filer")
grep -q "Bokföringen är obruten" "$WORK/insyn/index.html" && echo "  översikten: Bokföringen är obruten"

echo "== 9. Bokis på distans: agenten med kopplingskod och PKCE, och ett kvitto från telefonen"
FPORT=18771
coproc FJARR { exec "$PARTS/bok-fjarr" starta --adress "http://127.0.0.1:$FPORT" --port "$FPORT" 2>"$WORK/fjarr.err"; }
FPID=$FJARR_PID
trap 'kill "${RPID:-}" "${FPID:-}" 2>/dev/null || true' EXIT
for _ in $(seq 50); do
    curl -s -o /dev/null "http://127.0.0.1:$FPORT/.well-known/oauth-authorization-server" && break
    sleep 0.1
done
printf '%%PDF-1.4\n%% Kvitto 2026-311, betalt med kort\n' > "$WORK/kvitto-mobil.pdf"
python3 -I - "$PARTS/bok-fjarr" "$FPORT" "$WORK/kvitto-mobil.pdf" <<'EOF'
import base64, hashlib, json, re, subprocess, sys, urllib.error, urllib.parse, urllib.request

fjarr, port, receipt = sys.argv[1], sys.argv[2], sys.argv[3]
base = f"http://127.0.0.1:{port}"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


opener = urllib.request.build_opener(NoRedirect)


def call(path, data=None, headers=None):
    req = urllib.request.Request(base + path, data=data, headers=headers or {}, method="POST")
    try:
        with opener.open(req) as r:
            return r.status, r.headers, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read()


status, _, _ = call("/mcp", b"{}")
print(f"  /mcp utan nyckel: {status}")
_, _, body = call("/oauth/register",
                  json.dumps({"client_name": "e2e", "redirect_uris": ["http://localhost/callback"]}).encode(),
                  {"Content-Type": "application/json"})
client = json.loads(body)["client_id"]
out = subprocess.run([fjarr, "kod"], capture_output=True, text=True, check=True).stdout
code = re.search(r"Kopplingskod: (\S+)", out).group(1)
verifier = base64.urlsafe_b64encode(b"e2e-verifierare-" * 3).decode().rstrip("=")
challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip("=")
form = urllib.parse.urlencode({
    "response_type": "code", "client_id": client, "redirect_uri": "http://localhost:4711/callback",
    "code_challenge": challenge, "code_challenge_method": "S256", "state": "e2e", "kod": code}).encode()
status, headers, _ = call("/oauth/authorize", form,
                          {"Origin": base, "Content-Type": "application/x-www-form-urlencoded"})
location = headers["Location"]
auth = urllib.parse.parse_qs(urllib.parse.urlsplit(location).query)["code"][0]
print(f"  godkänd med kopplingskod: {status}, omdirigerad till {location.split('?')[0]}")
tokens = json.loads(call("/oauth/token", urllib.parse.urlencode({
    "grant_type": "authorization_code", "code": auth, "client_id": client,
    "redirect_uri": "http://localhost:4711/callback", "code_verifier": verifier}).encode())[2])
mcp = {"Authorization": "Bearer " + tokens["access_token"], "Content-Type": "application/json",
       "MCP-Protocol-Version": "2025-06-18"}
tools = json.loads(call("/mcp", json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).encode(), mcp)[2])
names = [t["name"] for t in tools["result"]["tools"]]
assert "foresla_verifikation" in names and "bokfor_forslag" not in names
print(f"  agentens verktyg: {len(names)}, inget för att bokföra")

out = subprocess.run([fjarr, "uppladdning", "telefon"], capture_output=True, text=True, check=True).stdout
code = re.search(r"^\s+([2-9A-HJ-NP-Z]{4}-[2-9A-HJ-NP-Z]{4})\s*$", out, re.M).group(1)
status, headers, _ = call("/", urllib.parse.urlencode({"kod": code.lower()}).encode(),
                          {"Origin": base, "Content-Type": "application/x-www-form-urlencoded"})
assert status == 303 and re.fullmatch(r"/in/[0-9a-f]{64}", headers["Location"])
link = base + headers["Location"]
print(f"  telefonen hämtade sin länk med koden på startsidan: {status}")
boundary = "e2e-boundary-41"
data = open(receipt, "rb").read()
body = (f"--{boundary}\r\nContent-Disposition: form-data; name=\"fil\"; filename=\"kvitto-mobil.pdf\"\r\n\r\n"
        .encode() + data + f"\r\n--{boundary}--\r\n".encode())
status, headers, _ = call(link.replace(base, ""), body,
                          {"Content-Type": f"multipart/form-data; boundary={boundary}", "Origin": base})
doc = re.search(r"mottaget=(\d+)", headers["Location"]).group(1)
print(f"  kvittot från telefonen blev underlag {doc}")
got = json.loads(call("/mcp", json.dumps({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
    "name": "hamta_underlag", "arguments": {"id": doc}}}).encode(), mcp)[2])
assert "kvitto-mobil.pdf" in got["result"]["content"][0]["text"]
print("  agenten hämtade underlaget över HTTP")
EOF
kill "$FPID"
"$BOK" historik | grep -E "fjarr\.(anslutning|lank)" | sed 's/^/  /'
"$BOK" verify | tail -1
