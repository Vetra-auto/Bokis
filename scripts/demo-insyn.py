#!/usr/bin/env python3
"""Bygger en demobokföring att visa insynen med: ett stängt räkenskapsår
(2025) och ett pågående (2026), med kvitton som bilder och PDF, fakturor som
text, förslag från en agent genom bok-mcp, bankavstämning, en rättelse,
momsavräkning, bokslut och ingående balans, ett flaggat dubblettkvitto och en
obesvarad fråga.

Allt skapas i en ny katalog med en engångsnyckel utan lösenord. Företaget och
alla motparter är påhittade.

    cargo build -p bok-cli -p bok-mcp
    python3 scripts/demo-insyn.py /tmp/bokis-demo
    target/debug/bok --db /tmp/bokis-demo/bok/bokis.db insyn

Kräver Pillow för kvittobilderna.
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
BOK = os.environ.get("BOK", str(ROOT / "target/debug/bok"))
MCP = os.environ.get("BOK_MCP", str(ROOT / "target/debug/bok-mcp"))
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"

AGENT = {"modell": "claude-opus-5-5", "leverantor": "Anthropic", "region": "eu"}


def bok(*args: str) -> str:
    out = subprocess.run([BOK, *args], capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"bok {' '.join(args)} misslyckades:\n{out.stdout}{out.stderr}")
    return out.stdout


def agent(calls: list) -> list:
    """Anrop till bok-mcp över stdio, som en agent gör. Ger svarstexterna."""
    msgs = [
        {"jsonrpc": "2.0", "id": 0, "method": "initialize",
         "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": {"name": "demo", "version": "1"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
    ]
    msgs += [{"jsonrpc": "2.0", "id": i + 1, "method": "tools/call",
              "params": {"name": n, "arguments": a}} for i, (n, a) in enumerate(calls)]
    out = subprocess.run([MCP, "--db", os.environ["BOKIS_DB"]],
                         input="\n".join(json.dumps(m) for m in msgs) + "\n",
                         capture_output=True, text=True, check=True).stdout
    texts = []
    for line in out.splitlines():
        r = json.loads(line)
        if r["id"] == 0:
            continue
        if "error" in r or r["result"].get("isError"):
            sys.exit(f"agenten nekades: {line}")
        texts.append(r["result"]["content"][0].get("text", ""))
    return texts


def receipt(path: Path, lines: list, width: int = 520, pdf: bool = False) -> Path:
    """Ett kvitto som bild: svart text på papper, lite snett som ett foto."""
    font = ImageFont.truetype(FONT, 22)
    small = ImageFont.truetype(FONT, 18)
    height = 90 + 34 * len(lines)
    img = Image.new("RGB", (width, height), (252, 251, 247))
    draw = ImageDraw.Draw(img)
    y = 40
    for i, line in enumerate(lines):
        f = font if i == 0 else small
        draw.text((36, y), line, fill=(30, 30, 30), font=f)
        y += 34
    if pdf:
        img.save(path, "PDF", resolution=110)
        return path
    photo = Image.new("RGB", (width + 80, height + 80), (214, 210, 200))
    photo.paste(img.rotate(1.2, expand=True, fillcolor=(214, 210, 200)), (30, 30))
    photo.save(path, "JPEG", quality=88)
    return path


def attach(path: Path, sender: str) -> str:
    out = bok("underlag", "lagg-till", str(path), "--sender", sender)
    return re.search(r"Underlag (\d+) mottaget", out).group(1)


def propose(date, text, lines, attachments=(), counterparty=None, uncertainty=0.08, why=""):
    """Ett förslag från agenten, som granskaren sedan godkänner."""
    args = {
        "affarshandelse_datum": date,
        "beskrivning": text,
        "rader": [{"konto": k, ("debet_ore" if ore > 0 else "kredit_ore"): abs(ore)} for k, ore in lines],
        "underlag_ids": list(attachments),
        "harkomst": {**AGENT, "motivering": why or "Enligt underlaget.", "osakerhet": uncertainty},
    }
    if counterparty:
        args["motpart"] = counterparty
    text = agent([["foresla_verifikation", args]])[0]
    proposal = re.search(r"Förslaget (\d+) är sparat", text).group(1)
    out = bok("godkann", proposal)
    return re.search(r"Bokförd som ([A-Z]\d+)", out).group(1)


def human(date, text, lines, attachments=()):
    args = ["foresla", "--date", date, "--text", text, "--approve"]
    for k, ore in lines:
        args += ["--line", f"{k}={ore / 100:.2f}".replace(".", ",")]
    for a in attachments:
        args += ["--attachment", a]
    return bok(*args)


def main() -> None:
    work = Path(sys.argv[1] if len(sys.argv) > 1 else sys.exit("ange en katalog som inte finns"))
    if work.exists():
        sys.exit(f"{work} finns redan")
    (work / "bok").mkdir(parents=True)
    files = work / "filer"
    files.mkdir()
    subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "bokis-demo",
                    "-f", str(work / "nyckel")], check=True)
    os.environ.update(BOKIS_DB=str(work / "bok/bokis.db"), BOKIS_KEY=str(work / "nyckel"),
                      BOKIS_USER="granskare")

    print("Bokföringen och kontoplanen")
    bok("init", "--security", "mjukvara", "--company", "Fjällvind Konsult AB", "--orgnr", "559999-0001",
        "--year", "2025")
    bok("ar", "add", "2026")
    chart = files / "konton.txt"
    chart.write_text("\n".join([
        "1510;Kundfordringar", "1630;Skattekonto", "1930;Företagskonto",
        "2081;Aktiekapital", "2091;Balanserad vinst eller förlust", "2099;Årets resultat",
        "2440;Leverantörsskulder", "2611;Utgående moms på försäljning inom Sverige, 25 %",
        "2614;Utgående moms omvänd betalningsskyldighet, 25 %", "2641;Debiterad ingående moms",
        "2645;Beräknad ingående moms på förvärv från utlandet", "2650;Redovisningskonto för moms",
        "3001;Försäljning inom Sverige, 25 % moms", "4535;Inköp av tjänster från annat EU-land, 25 %",
        "5410;Förbrukningsinventarier", "5420;Programvaror", "5800;Resekostnader",
        "6071;Representation, avdragsgill", "6212;Mobiltelefon", "6530;Redovisningstjänster",
        "6570;Bankkostnader", "8999;Årets resultat",
    ]) + "\n")
    bok("konto", "import", str(chart), "--vat-codes", str(ROOT / "docs/momskoder-bas.txt"),
        "--from", "2025-01-01")

    print("2025: aktiekapital, köp, försäljning, representation och tjänst från EU")
    bank_letter = files / "bankbesked-aktiekapital.txt"
    bank_letter.write_text("Norrlandsbanken\nBesked om insättning av aktiekapital\n"
                           "Fjällvind Konsult AB (under bildande)\nBelopp 25 000,00 kr\n"
                           "Datum 2025-01-03\n")
    u = attach(bank_letter, "banken")
    human("2025-01-03", "Insättning av aktiekapital", [(1930, 2_500_000), (2081, -2_500_000)], [u])

    u = attach(receipt(files / "kvitto-kodverkstan.jpg", [
        "KODVERKSTAN AB", "org.nr 556000-1111", "Kvitto 2025-02-10 14:02", "",
        "Utvecklarlicens, 1 år     1 200,00", "Moms 25 %                  300,00",
        "Totalt                   1 500,00", "Betalt med kort ****4417"]), "kvitton@fjallvind.example")
    propose("2025-02-10", "Utvecklarlicens för ett år, Kodverkstan AB",
            [(5420, 120_000), (2641, 30_000), (1930, -150_000)], [u],
            {"namn": "Kodverkstan AB", "orgnr": "556000-1111"},
            why="Kvittot visar en programlicens på ett år med 25 % moms, betald med företagets kort.")

    invoice = files / "faktura-1001.txt"
    invoice.write_text("Fjällvind Konsult AB\nFaktura 1001, 2025-03-31\nTill Nordlund Fastigheter AB\n"
                       "Konsulttjänster mars, 50 timmar à 800 kr     40 000,00\nMoms 25 %   10 000,00\n"
                       "Att betala 50 000,00 kr senast 2025-04-30\n")
    u = attach(invoice, "faktura@fjallvind.example")
    propose("2025-03-31", "Faktura 1001 till Nordlund Fastigheter AB, konsulttjänster mars",
            [(1510, 5_000_000), (3001, -4_000_000), (2611, -1_000_000)], [u],
            {"namn": "Nordlund Fastigheter AB", "orgnr": "556000-2222"},
            why="Utgående faktura för 50 timmar konsulttjänster med 25 % moms.")
    propose("2025-04-28", "Inbetalning av faktura 1001 från Nordlund Fastigheter AB",
            [(1930, 5_000_000), (1510, -5_000_000)], [],
            {"namn": "Nordlund Fastigheter AB"}, why="Bankraden matchar faktura 1001 till beloppet.")

    u = attach(receipt(files / "kvitto-tallen.jpg", [
        "RESTAURANG TALLEN", "Storgatan 4, Umeå", "2025-06-12 12:41", "",
        "2 x Dagens lunch            1 008,00", "  varav moms 12 %             108,00",
        "Totalt                     1 008,00", "Kort ****4417"]), "kvitton@fjallvind.example")
    case = agent([["stall_fraga", {"underlag_id": u, "fraga": "Vilka deltog i lunchen och vad var syftet?",
                                   "harkomst": {**AGENT, "motivering": "Representation kräver deltagare och syfte.",
                                                "osakerhet": 0.3}}]])[0]
    bok("arende", "svara", re.search(r"(\d+)", case).group(1),
        "Lunch med Nordlund Fastigheter AB om uppdraget hösten 2025, två deltagare.")
    propose("2025-06-12", "Representation, lunch med Nordlund Fastigheter AB",
            [(6071, 90_000), (2641, 10_800), (1930, -100_800)], [u],
            {"namn": "Restaurang Tallen"}, uncertainty=0.15,
            why="Lunch med kund, två deltagare enligt svaret på frågan. Moms 12 % på livsmedel.")

    u = attach(receipt(files / "invoice-cloudsketch.pdf", [
        "CLOUDSKETCH LTD", "Dublin, Ireland  VAT IE1234567X", "Invoice CS-88231, 2025-09-15", "",
        "Design workspace, annual    SEK 1 000,00", "VAT reverse charge (art. 196)",
        "Customer VAT SE559999000101", "Total                       SEK 1 000,00"], pdf=True),
        "billing@cloudsketch.example")
    propose("2025-09-15", "Designverktyg, årsabonnemang från Cloudsketch Ltd (Irland)",
            [(4535, 100_000), (2645, 25_000), (2614, -25_000), (1930, -100_000)], [u],
            {"namn": "Cloudsketch Ltd", "momsnr": "IE1234567X"},
            why="Tjänst från annat EU-land med omvänd betalningsskyldighet: ruta 21, 30 och 48.")

    vat_doc = files / "momsdeklaration-2025.txt"
    vat_doc.write_text("Momsdeklaration 2025 (helår)\nRuta 05 40 000\nRuta 10 10 000\nRuta 21 1 000\n"
                       "Ruta 30 250\nRuta 48 658\nRuta 49 9 592\n")
    u = attach(vat_doc, "skatteverket")
    human("2025-12-31", "Momsavräkning 2025", [(2611, 1_000_000), (2614, 25_000), (2641, -40_800),
                                              (2645, -25_000), (2650, -959_200)], [u])

    print("2025: bank, avstämning, bokslut och stängning")
    csv = files / "kontoutdrag-2025.csv"
    csv.write_text("datum;text;belopp;saldo\n2025-01-03;Insättning aktiekapital;25000,00;25000,00\n"
                   "2025-02-10;KODVERKSTAN AB;-1500,00;23500,00\n2025-04-28;NORDLUND FASTIGHETER;50000,00;73500,00\n"
                   "2025-06-12;RESTAURANG TALLEN;-1008,00;72492,00\n2025-09-15;CLOUDSKETCH LTD;-1000,00;71492,00\n")
    bok("bank", "importera", str(csv), "--csv-account", "SE45 5000 0000 0583 9825 7466")
    for row, ver in [(1, "A1"), (2, "A2"), (3, "A4"), (4, "A5"), (5, "A6")]:
        bok("bank", "matcha", str(row), ver, "--year", "2025")
    bok("avstam", "1930", "--date", "2025-12-31", "--balance", "71492,00", "--note", "Kontoutdrag december")
    bok("avstam", "1630", "--date", "2025-12-31", "--balance", "0", "--note", "Skattekontot enligt Skatteverket")
    out = bok("bokslut", "resultat", "--year", "2025")
    bok("godkann", re.search(r"[Ff]örslag (\d+)", out).group(1))
    for m in range(1, 13):
        bok("period", "granska", f"2025-{m:02d}")
        bok("period", "stang", f"2025-{m:02d}")
    out = bok("bokslut", "ib", "--year", "2025")
    bok("godkann", re.search(r"[Ff]örslag (\d+)", out).group(1))

    print("2026: moms, mobil, kontorsstol med rättelse, faktura, resa och redovisning")
    human("2026-01-20", "Betalning av moms för 2025", [(2650, 959_200), (1930, -959_200)])
    u = attach(receipt(files / "kvitto-mobilnat.jpg", [
        "MOBILNÄT NORR AB", "Faktura 2026-02", "Abonnemang februari", "",
        "Mobilabonnemang              400,00", "Moms 25 %                    100,00",
        "Att betala                   500,00"]), "faktura@mobilnat.example")
    propose("2026-02-05", "Mobilabonnemang februari, Mobilnät Norr AB",
            [(6212, 40_000), (2641, 10_000), (1930, -50_000)], [u], {"namn": "Mobilnät Norr AB"})
    chair = receipt(files / "kvitto-kontorshuset.jpg", [
        "KONTORSHUSET I UMEÅ AB", "Kvitto 2026-03-03", "", "Kontorsstol Ergo 2       3 196,00",
        "Moms 25 %                    799,00", "Totalt                     3 995,00", "Kort ****4417"])
    u = attach(chair, "kvitton@fjallvind.example")
    wrong = propose("2026-03-03", "Kontorsstol, Kontorshuset i Umeå AB",
                    [(5420, 319_600), (2641, 79_900), (1930, -399_500)], [u],
                    {"namn": "Kontorshuset i Umeå AB"}, uncertainty=0.35,
                    why="Inköp enligt kvittot; kontot valt efter leverantörens tidigare fakturor.")
    bok("ratta", wrong, "--year", "2026", "--reason", "Kontorsstolen är en inventarie, inte programvara",
        "--line", "5410=3196,00", "--line", "2641=799,00", "--line", "1930=-3995,00", "--approve")
    invoice = files / "faktura-1007.txt"
    invoice.write_text("Fjällvind Konsult AB\nFaktura 1007, 2026-03-31\nTill Nordlund Fastigheter AB\n"
                       "Konsulttjänster mars, 75 timmar à 800 kr     60 000,00\nMoms 25 %   15 000,00\n"
                       "Att betala 75 000,00 kr senast 2026-04-30\n")
    u = attach(invoice, "faktura@fjallvind.example")
    propose("2026-03-31", "Faktura 1007 till Nordlund Fastigheter AB, konsulttjänster mars",
            [(1510, 7_500_000), (3001, -6_000_000), (2611, -1_500_000)], [u],
            {"namn": "Nordlund Fastigheter AB", "orgnr": "556000-2222"})
    propose("2026-04-25", "Inbetalning av faktura 1007 från Nordlund Fastigheter AB",
            [(1930, 7_500_000), (1510, -7_500_000)], [], {"namn": "Nordlund Fastigheter AB"})
    u = attach(receipt(files / "biljett-norrlandstag.jpg", [
        "NORRLANDSTÅG AB", "Umeå C - Stockholm C, 2026-05-14", "Resenär: 1 vuxen", "",
        "Biljett                    1 339,00", "  varav moms 6 %               75,79",
        "Totalt                     1 339,00"]), "kvitton@fjallvind.example")
    propose("2026-05-14", "Tågresa Umeå–Stockholm för kundmöte",
            [(5800, 126_321), (2641, 7_579), (1930, -133_900)], [u], {"namn": "Norrlandståg AB"},
            why="Persontransport med 6 % moms; resan gäller kundmötet i Stockholm.")
    human("2026-06-30", "Bankavgifter andra kvartalet", [(6570, 30_000), (1930, -30_000)])
    u = attach(receipt(files / "faktura-siffror-saldo.pdf", [
        "SIFFROR & SALDO AB", "Faktura 5521, 2026-08-19", "Bokslut och årsredovisning 2025", "",
        "Arvode                     2 400,00", "Moms 25 %                    600,00",
        "Att betala 3 000,00 senast 2026-09-18"], pdf=True), "faktura@siffror.example")
    propose("2026-08-19", "Bokslut och årsredovisning 2025, Siffror & Saldo AB",
            [(6530, 240_000), (2641, 60_000), (2440, -300_000)], [u],
            {"namn": "Siffror & Saldo AB", "orgnr": "556000-3333"})
    propose("2026-09-10", "Betalning av faktura 5521 till Siffror & Saldo AB",
            [(2440, 300_000), (1930, -300_000)], [], {"namn": "Siffror & Saldo AB"})

    print("2026: dubblettkvitto, obesvarad fråga, bank och stängda perioder")
    again = files / "kvitto-kontorshuset-igen.jpg"
    again.write_bytes(chair.read_bytes() + b"\x00")
    u = attach(again, "kvitton@fjallvind.example")
    flag = agent([["flagga_underlag", {"underlag_id": u, "orsak": "dubblett",
                                       "kommentar": "Samma kontorsstol som på A3, skickad en gång till.",
                                       "harkomst": {**AGENT, "motivering": "Samma belopp, datum och butik.",
                                                    "osakerhet": 0.05}}]])[0]
    bok("arende", "svara", re.search(r"(\d+)", flag).group(1), "Ja, samma kvitto.", "--mark", "dubblett")
    u = attach(receipt(files / "kvitto-okant.jpg", [
        "SWISH", "2026-10-02 09:15", "Mottagare: 123 456 78 90", "", "Belopp 250,00"]),
        "kvitton@fjallvind.example")
    agent([["stall_fraga", {"underlag_id": u, "fraga": "Vad gällde Swish-betalningen på 250 kr den 2 oktober?",
                            "harkomst": {**AGENT, "motivering": "Kvittot säger inte vad som köptes.",
                                         "osakerhet": 0.9}}]])
    csv = files / "kontoutdrag-2026.csv"
    csv.write_text("datum;text;belopp;saldo\n2026-01-20;SKATTEVERKET MOMS;-9592,00;61900,00\n"
                   "2026-02-05;MOBILNAT NORR;-500,00;61400,00\n2026-03-03;KONTORSHUSET UMEA;-3995,00;57405,00\n"
                   "2026-04-25;NORDLUND FASTIGHETER;75000,00;132405,00\n2026-05-14;NORRLANDSTAG;-1339,00;131066,00\n"
                   "2026-06-30;AVGIFTER;-300,00;130766,00\n2026-09-10;SIFFROR OCH SALDO;-3000,00;127766,00\n"
                   "2026-10-02;SWISH 1234567890;-250,00;127516,00\n")
    bok("bank", "importera", str(csv), "--csv-account", "SE45 5000 0000 0583 9825 7466")
    for row, ver in [(6, "A1"), (7, "A2"), (8, "A3"), (9, "A5"), (10, "A6"), (11, "A7"), (12, "A9")]:
        bok("bank", "matcha", str(row), ver, "--year", "2026")
    for m in range(1, 7):
        bok("period", "granska", f"2026-{m:02d}")
        bok("period", "stang", f"2026-{m:02d}")
    bok("avstam", "1930", "--date", "2026-06-30", "--balance", "130766,00", "--note", "Kontoutdrag juni")
    print(bok("verify").strip().splitlines()[-1])
    print(f"Klart. Starta insynen med:\n  {BOK} --db {work / 'bok/bokis.db'} insyn")


if __name__ == "__main__":
    main()
