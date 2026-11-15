# Bokis på distans: agenten i mobilen och kvittofoton

Med `bok-fjarr` bokför du från telefonen. Du fotograferar kvittot och skickar det till Bokis med en
personlig länk. Sedan ber du Claude, ChatGPT eller Claude Code att bokföra det. Agenten ansluter
över HTTPS och får samma verktyg som lokalt: den kan läsa och föreslå men aldrig bokföra. Du
godkänner förslaget med nyckeln som vanligt.

```bash
cargo build --release -p bok-fjarr
bok-fjarr starta --adress https://bokis.ditt-tailnet.ts.net   # lyssnar på 127.0.0.1:8770
tailscale funnel --bg 8770        # publik adress; krävs för Claude i webbläsaren och mobilappen
bok-fjarr kod                     # kopplingskod för en ny anslutning, gäller 15 minuter och en gång
bok-fjarr uppladdning telefon     # kod som telefonen hämtar sin personliga länk med
bok-fjarr uppladdning ipad --lank # hela länken direkt, för att skicka den på annat sätt
bok-fjarr lista                   # anslutningar och länkar, med senaste användning
bok-fjarr aterkalla telefon       # gäller direkt; loggas i behandlingshistoriken
```

**Lägga till Bokis i Claude.** Gå till Inställningar, Anslutningar, Lägg till anpassad anslutning
(Settings, Connectors, Add custom connector) och ange `https://bokis.ditt-tailnet.ts.net/mcp`. Claude
registrerar sig själv och öppnar Bokis godkännandesida, där du skriver in koden från
`bok-fjarr kod`. Claude Code ansluter med
`claude mcp add --transport http bokis https://bokis.ditt-tailnet.ts.net/mcp`; logga in med `/mcp`.

**Telefonen.** Kör `bok-fjarr uppladdning telefon` på datorn. Öppna adressen, till exempel
`https://bokis.ditt-tailnet.ts.net`, i telefonen och skriv koden som visas. Telefonen skickas då till
sin personliga länk; lägg den på hemskärmen. Koden gäller i 15 minuter, en gång och i högst 5 försök.
Länken skapas först när koden används, så den visas aldrig på datorn. Startsidan säger inte vems
bokföring det är. Uppladdningssidan tar emot foton och PDF och sparar dem oförändrade. Den visar också vad som hänt med de senaste: om de väntar på
agenten, om ett förslag väntar på dig, eller vilken verifikation de bokfördes som. Sedan skriver du till
agenten, till exempel "Bokför underlag 17".

**Säkerheten:**
- **Nätverket.** Servern lyssnar bara på 127.0.0.1, och Tailscale sköter TLS och publiceringen.
  `tailscale funnel` gör adressen nåbar från internet. Det krävs för Claude i webbläsaren och
  mobilappen, som ansluter från Anthropics moln. `tailscale serve` håller den inom ditt tailnet,
  vilket räcker för Claude Code på datorn. Valet är ditt.
- **Godkännande.** Ingen anslutning godkänns utan kopplingskoden, som bara visas på datorn med
  bokföringen. Koden gäller i 15 minuter, en gång och i högst 5 försök.
- **Inloggning och nycklar.** Inloggningen är OAuth 2.1 med PKCE (S256) och dynamisk registrering.
  Nycklarna gäller en timme, och förnyelsenycklarna byts ut vid varje förnyelse.
- **Lagring och logg.** Nycklar, koder och länkar sparas bara som SHA-256 i `fjarr.json`, bredvid databasen
  med behörigheten 0600. Godkännanden, nya länkar och återkallelser loggas i behandlingshistoriken
  och syns i insynen och systemdokumentationen.
- **Webbskydd.** Servern kontrollerar Host och Origin. Sidorna har inget JavaScript och har en CSP.
  En uppladdning tas bara emot som bild eller PDF, avgjort av innehållet, med högst 25 MB per fil.
  Länken kontrolleras innan uppladdningen läses, så den som saknar en länk får skicka högst 1 MB.
- **Bilder till agenten.** Agenten får kvittofoton nedskalade till högst 1 568 bildpunkter och
  100 kB, med rätt orientering, eftersom ett verktygssvar i Claude får vara ungefär 150 000 tecken.
  Originalet ligger orört i arkivet.

En kontroll i webbläsaren hittade samma fel som i granskningsvyn: med `Referrer-Policy: no-referrer`
skickar webbläsaren `Origin: null`, och godkännandet nekades. Sidorna med formulär har nu
`same-origin`, och ett test låser det.

**Begränsningar:**
- Datorn måste vara igång och vaken för att agenten och telefonen ska nå Bokis, om inte Bokis körs
  på en Raspberry Pi (se nedan).
- Agenten får inte se en HEIC-bild. Safari på iPhone skickar JPEG från sidan, men en HEIC-fil från
  appen Filer sparas som den är, och agenten får be om en JPEG.
- En PDF skickas till agenten som fil. Claude i webbläsaren och mobilappen läser bara text och
  bilder i verktygssvar, så en PDF-faktura går bättre genom Claude Code.
- Bygg med `--release`, eftersom nedskalningen av foton är långsam i en debugbyggnad.

## På en Raspberry Pi

En Raspberry Pi 3 eller nyare med 64-bitars Raspberry Pi OS räcker. Programmen är några MB och
servern använder runt 10 MB minne. När agenten hämtar ett mobilfoto på 12 MP tar nedskalningen
upp mot tio sekunder och runt 130 MB minne. Pi:n är alltid på, så telefonen och agenten når Bokis
även när datorn är avstängd.

I exemplen är `pi` Pi:ns namn. `ssh pi` loggar in på ditt eget konto, som har sudo, och
`ssh bokis@pi` på kontot som bokföringen körs under.

**Bygg på datorn.** Pi:n har för lite minne för att kompilera arbetsytan. Med zig (0.17) och
cargo-zigbuild blir programmen statiska filer som går att kopiera rakt över:

```bash
rustup target add aarch64-unknown-linux-musl
cargo install --locked cargo-zigbuild
cargo zigbuild --release --target aarch64-unknown-linux-musl -p bok-cli -p bok-fjarr -p bok-mcp -p bok-review
scp target/aarch64-unknown-linux-musl/release/{bok,bok-fjarr,bok-mcp,bok-review} pi:
ssh -t pi sudo install -m 0755 bok bok-fjarr bok-mcp bok-review /usr/local/bin/
```

**Förbered Pi:n.** Skriv Raspberry Pi OS Lite (64-bit) till SD-kortet med Raspberry Pi Imager och
ställ där in värdnamn, ditt konto med SSH-nyckel och tidszon. `uname -m` ska svara `aarch64`.
Använd den officiella nätdelen, eftersom SSD:n får sin ström genom Pi:n.

Lägg databasen på en SSD i ett USB-kabinett, inte på SD-kortet eller en USB-sticka. De slits ut och
kan tappa det senast skrivna vid ett strömavbrott. Bokföringen får ett eget konto utan sudo, så den
som tar sig in genom bok-fjarr inte kommer åt resten av Pi:n. Tjänsten och dina kommandon kör som
samma konto, eftersom bara ägaren kan läsa databasen. `mkfs` raderar allt på disken, så kontrollera
först med `lsblk` att `/dev/sda1` är SSD:n:

```bash
sudo adduser --disabled-password --gecos "" bokis
sudo install -d -m 700 -o bokis -g bokis /home/bokis/.ssh
sudo install -m 600 -o bokis -g bokis ~/.ssh/authorized_keys /home/bokis/.ssh/
sudo mkfs.ext4 -L bokis /dev/sda1
sudo mkdir /mnt/bokis
echo 'LABEL=bokis /mnt/bokis ext4 defaults,noatime,nofail 0 2' | sudo tee -a /etc/fstab
sudo systemctl daemon-reload && sudo mount /mnt/bokis && sudo chown bokis: /mnt/bokis
printf 'BOKIS_DB=/mnt/bokis/bok/bokis.db\nBOKIS_USER=henrik\n' | sudo tee -a /etc/environment
```

`nofail` låter Pi:n starta även utan SSD:n, så att du fortfarande kommer åt den. `/etc/environment`
gäller både för tjänsten och för kommandon över SSH. `BOKIS_USER` är ditt namn i
behandlingshistoriken.

**Signera från datorn.** `bok` och `bok-review` signerar med `ssh-keygen -Y sign` på den maskin där
bokföringen ligger. ssh-keygen kan låta en vidarebefordrad ssh-agent göra själva signaturen. Då
sitter YubiKeyn kvar i datorn och Pi:n får bara den publika nyckeln.

Skicka inte vidare din vanliga agent med `ssh -A`. Den har nycklar till dina andra maskiner, och
tjänsten som nås från internet kör som samma konto som dina kommandon. Funktionen `signera` startar
i stället en egen agent med bara bokföringsnyckeln och skickar vidare den. Lägg den i `~/.bashrc`:

```bash
signera() {
  HUVUD="$SSH_AUTH_SOCK" ssh-agent sh -c 'ssh-add -q ~/.ssh/id_ed25519_sk &&
    ssh -o IdentityAgent="$HUVUD" -o ForwardAgent="$SSH_AUTH_SOCK" "$@"' sh "$@"
}
```

```bash
scp ~/.ssh/id_ed25519_sk.pub bokis@pi:.ssh/
signera bokis@pi bok godkann 17
signera -L 8484:127.0.0.1:8484 bokis@pi bok-review --db /mnt/bokis/bok/bokis.db --granskare henrik
```

Inloggningen går som vanligt genom din vanliga agent, men Pi:n ser bara bokföringsnyckeln. På
säkerhetsnivån `mjukvara` byter du `id_ed25519_sk` mot din nyckelfil.
bok-review läser varken `BOKIS_DB` eller `BOKIS_USER`, därav `--db` och `--granskare`. Varje
signatur kräver en beröring av nyckeln. Medan en sådan session är öppen kan den som har root på Pi:n,
eller har tagit sig in i bokis-kontot, be agenten om signaturer. Rör därför bara nyckeln när du själv
har bett om en signatur. Vägen genom agenten är provad med en mjukvarunyckel, men inte med en
YubiKey.

**Bokföringen.** Skapa en ny bokföring på Pi:n, eller flytta den från datorn med en kopia. Kopian
läses in i en ny katalog och kontrolleras innan den används:

```bash
signera bokis@pi 'bok init --company "…" --orgnr … --year 2026'
```

```bash
bok kopia skapa ~/flytt          # på datorn
scp -r ~/flytt bokis@pi:
ssh bokis@pi 'bok kopia las-in flytt/kopior/<tid> /mnt/bokis/bok'
```

Flytta hela katalogen: underlagen ligger bredvid kopian, i `~/flytt/underlag`.

Pi:n och SSD:n är en enda plats, så ta ändå kopior till en annan maskin med `bok kopia skapa`.

**Tjänsten.** Installera Tailscale på Pi:n och logga in med `sudo tailscale up`. Slå på Funnel där
och stäng den på datorn. Godkänn Funnel för Pi:n på Tailscales adminsida om kommandot ber om det.
`tailscale funnel status` visar adressen som ska stå i tjänsten:

```bash
sudo tailscale funnel --bg 8770    # på Pi:n
sudo tailscale funnel reset        # på datorn
```

Pi:n har ingen batteriklocka. Efter ett strömavbrott går den med gammal tid tills nätet har ställt
den. Bokis skriver ingen händelse med en tid före den senaste i historiken, men starta ändå tjänsten
först när klockan är ställd. Slå på `systemd-time-wait-sync`, annars väntar `time-sync.target` inte
på något. `RequiresMountsFor` håller tjänsten nere när SSD:n saknas:

```ini
# /etc/systemd/system/bok-fjarr.service
[Unit]
Description=Bokis på distans
Wants=time-sync.target network-online.target
After=time-sync.target network-online.target
RequiresMountsFor=/mnt/bokis

[Service]
User=bokis
EnvironmentFile=/etc/environment
ExecStart=/usr/local/bin/bok-fjarr starta --adress https://<pi>.<tailnet>.ts.net
Restart=on-failure
PrivateTmp=yes
NoNewPrivileges=yes

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable --now systemd-time-wait-sync bok-fjarr
systemctl status bok-fjarr
```

Adressen är ny, så Claude och telefonen kopplas på nytt. Lägg till
`https://<pi>.<tailnet>.ts.net/mcp` som anslutning i Claude och godkänn den med koden från
`ssh bokis@pi bok-fjarr kod`. Telefonen öppnar startsidan och skriver koden från
`ssh bokis@pi bok-fjarr uppladdning telefon`.

Om den senaste händelsen fick fel tid, för att klockan gick före när den skrevs, godtar du det med
`bok klocka godta --reason "…"`. Händelsen och förklaringen syns i historiken.

**Insynen** kontrollerar arkivet en gång per start och inte för varje sida. På en Pi 3 läses
underlagen med runt 30 MB/s, så kontrollen tar ungefär en halv minut per GB underlag. Öppna den
genom en tunnel med samma port på båda sidor:

```bash
ssh -L 8765:127.0.0.1:8765 bokis@pi bok insyn --port 8765
```

**Uppdatera.** Ta en kopia, bygg och installera filerna som ovan och starta om tjänsten med
`ssh -t pi sudo systemctl restart bok-fjarr`.

**Provkör utan Pi.** Med qemu-user går aarch64-filerna att köra på datorn. Hela testsviten körs då
på aarch64, utom bok-cli:s tester. De startar programmet självt och kräver därför qemu-user-binfmt:

```bash
sudo apt install qemu-user
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUNNER=qemu-aarch64 cargo-zigbuild test \
  --target aarch64-unknown-linux-musl -p bok-core -p bok-store -p bok-archive -p bok-bank \
  -p bok-backup -p bok-intake -p bok-mcp -p bok-review -p bok-eval -p bok-fjarr -p bok-insyn
```

Kvar enligt arkitekturen: härdningen i steg 6
(`bokd` som ensam ägare av databasen, RFC 3161-tidsstämplar, återläsningsövning som rutin). Steg 6
hänger på öppna frågor i arkitekturen.
