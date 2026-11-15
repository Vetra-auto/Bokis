-- Bokis AI-first, schema version 1.
--
-- Allt som är räkenskapsinformation kan bara växa. Triggers stoppar UPDATE
-- och DELETE på de tabellerna, och stoppar INSERT OR REPLACE genom att vägra
-- en ny rad med samma nyckel som en gammal (DELETE-triggers körs inte vid
-- REPLACE om recursive_triggers är av). Varje skrivning sker i en
-- transaktion (BEGIN IMMEDIATE) som också lägger en hashkedjad händelse i
-- `handelser`.
--
-- Kontrollen mot ersättning görs med EXISTS på id. Att jämföra NEW.id med
-- MAX(id) fungerar inte: i en BEFORE INSERT-trigger har en rad utan angivet id
-- ett platshållar-id (-1), så varje vanlig insättning skulle vägras.
--
-- Datum lagras som 'YYYY-MM-DD' och tidpunkter i UTC som
-- 'YYYY-MM-DDTHH:MM:SS.ffffffZ'. Belopp lagras i öre.

-- ---------------------------------------------------------------- handelser
-- Behandlingshistorik: vem, roll, vad, när, gamla och nya värden,
-- programversion, regelversion, hash och föregående hash.
CREATE TABLE handelser (
    seq             INTEGER PRIMARY KEY CHECK (seq >= 1),
    tid             TEXT NOT NULL CHECK (tid GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T*Z'),
    aktor_typ       TEXT NOT NULL CHECK (aktor_typ IN ('manniska', 'agent', 'system')),
    aktor           TEXT NOT NULL,
    roll            TEXT NOT NULL,
    handling        TEXT NOT NULL,
    objekt          TEXT,
    gamla           TEXT,
    nya             TEXT,
    programversion  TEXT NOT NULL,
    regelversion    TEXT,
    signatur        TEXT,
    foregaende_hash TEXT NOT NULL CHECK (length(foregaende_hash) = 64),
    hash            TEXT NOT NULL UNIQUE CHECK (length(hash) = 64)
) STRICT;

CREATE TRIGGER handelser_ny BEFORE INSERT ON handelser
BEGIN
    SELECT RAISE(ABORT, 'handelser: händelsen måste ha nästa nummer i följden')
    WHERE NEW.seq IS NOT (SELECT COALESCE(MAX(seq), 0) + 1 FROM handelser);
    SELECT RAISE(ABORT, 'handelser: föregående hash stämmer inte med kedjan')
    WHERE NEW.foregaende_hash IS NOT COALESCE(
        (SELECT hash FROM handelser ORDER BY seq DESC LIMIT 1),
        '0000000000000000000000000000000000000000000000000000000000000000');
    SELECT RAISE(ABORT, 'handelser: hashen finns redan')
    WHERE EXISTS (SELECT 1 FROM handelser WHERE hash = NEW.hash);
END;
CREATE TRIGGER handelser_ingen_andring BEFORE UPDATE ON handelser
BEGIN
    SELECT RAISE(ABORT, 'handelser: historiken kan bara växa');
END;
CREATE TRIGGER handelser_ingen_radering BEFORE DELETE ON handelser
BEGIN
    SELECT RAISE(ABORT, 'handelser: historiken kan bara växa');
END;

-- ------------------------------------------------------------------ foretag
CREATE TABLE foretag (
    id          INTEGER PRIMARY KEY,
    namn        TEXT NOT NULL CHECK (length(trim(namn)) > 0),
    orgnr       TEXT NOT NULL CHECK (length(trim(orgnr)) > 0),
    registrerad TEXT NOT NULL
) STRICT;

CREATE TRIGGER foretag_ny BEFORE INSERT ON foretag
BEGIN
    SELECT RAISE(ABORT, 'foretag: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM foretag WHERE id = NEW.id);
END;
CREATE TRIGGER foretag_ingen_andring BEFORE UPDATE ON foretag
BEGIN
    SELECT RAISE(ABORT, 'foretag: uppgifterna kan bara växa');
END;
CREATE TRIGGER foretag_ingen_radering BEFORE DELETE ON foretag
BEGIN
    SELECT RAISE(ABORT, 'foretag: uppgifterna kan bara växa');
END;

-- ------------------------------------------------------------------ nycklar
-- Publika nycklar som får signera bokföring, periodbeslut och systemändringar.
CREATE TABLE nycklar (
    id            INTEGER PRIMARY KEY,
    namn          TEXT NOT NULL CHECK (length(trim(namn)) > 0),
    publik_nyckel TEXT NOT NULL,
    fingeravtryck TEXT NOT NULL UNIQUE,
    algoritm      TEXT NOT NULL,
    hardvara      INTEGER NOT NULL CHECK (hardvara IN (0, 1)),
    registrerad   TEXT NOT NULL,
    handelse      INTEGER NOT NULL REFERENCES handelser(seq) DEFERRABLE INITIALLY DEFERRED
) STRICT;

CREATE TRIGGER nycklar_ny BEFORE INSERT ON nycklar
BEGIN
    SELECT RAISE(ABORT, 'nycklar: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM nycklar WHERE id = NEW.id);
    SELECT RAISE(ABORT, 'nycklar: nyckeln är redan registrerad')
    WHERE EXISTS (SELECT 1 FROM nycklar WHERE fingeravtryck = NEW.fingeravtryck);
END;
CREATE TRIGGER nycklar_ingen_andring BEFORE UPDATE ON nycklar
BEGIN
    SELECT RAISE(ABORT, 'nycklar: en registrerad nyckel kan inte ändras');
END;
CREATE TRIGGER nycklar_ingen_radering BEFORE DELETE ON nycklar
BEGIN
    SELECT RAISE(ABORT, 'nycklar: en registrerad nyckel kan inte raderas');
END;

-- ------------------------------------------------------------- rakenskapsar
CREATE TABLE rakenskapsar (
    id          INTEGER PRIMARY KEY,
    start       TEXT NOT NULL UNIQUE CHECK (date(start) IS start),
    slut        TEXT NOT NULL CHECK (date(slut) IS slut),
    registrerad TEXT NOT NULL,
    CHECK (start < slut)
) STRICT;

CREATE TRIGGER rakenskapsar_ny BEFORE INSERT ON rakenskapsar
BEGIN
    SELECT RAISE(ABORT, 'rakenskapsar: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM rakenskapsar WHERE id = NEW.id);
    SELECT RAISE(ABORT, 'rakenskapsar: räkenskapsåret överlappar ett annat')
    WHERE EXISTS (SELECT 1 FROM rakenskapsar WHERE NOT (NEW.slut < start OR NEW.start > slut));
END;
CREATE TRIGGER rakenskapsar_ingen_andring BEFORE UPDATE ON rakenskapsar
BEGIN
    SELECT RAISE(ABORT, 'rakenskapsar: räkenskapsåret kan inte ändras');
END;
CREATE TRIGGER rakenskapsar_ingen_radering BEFORE DELETE ON rakenskapsar
BEGIN
    SELECT RAISE(ABORT, 'rakenskapsar: räkenskapsåret kan inte raderas');
END;

-- ----------------------------------------------------------------- perioder
-- Öppen, under avstämning, stängd eller låst. Stängning och låsning är
-- signerade beslut (signaturen ligger i händelsen). Stängd och låst kan
-- inte öppnas.
CREATE TABLE perioder (
    id              INTEGER PRIMARY KEY,
    rakenskapsar_id INTEGER NOT NULL REFERENCES rakenskapsar(id),
    start           TEXT NOT NULL UNIQUE CHECK (date(start) IS start),
    slut            TEXT NOT NULL CHECK (date(slut) IS slut),
    status          TEXT NOT NULL DEFAULT 'oppen'
                    CHECK (status IN ('oppen', 'under_avstamning', 'stangd', 'last')),
    CHECK (start <= slut)
) STRICT;

CREATE TRIGGER perioder_ny BEFORE INSERT ON perioder
BEGIN
    SELECT RAISE(ABORT, 'perioder: raden finns redan och kan inte ersättas')
    WHERE (EXISTS (SELECT 1 FROM perioder WHERE id = NEW.id))
       OR EXISTS (SELECT 1 FROM perioder WHERE start = NEW.start);
    SELECT RAISE(ABORT, 'perioder: en ny period är öppen')
    WHERE NEW.status IS NOT 'oppen';
    SELECT RAISE(ABORT, 'perioder: perioden ligger utanför räkenskapsåret')
    WHERE NEW.start < (SELECT start FROM rakenskapsar WHERE id = NEW.rakenskapsar_id)
       OR NEW.slut > (SELECT slut FROM rakenskapsar WHERE id = NEW.rakenskapsar_id);
END;
CREATE TRIGGER perioder_status BEFORE UPDATE ON perioder
BEGIN
    SELECT RAISE(ABORT, 'perioder: bara status får ändras')
    WHERE NEW.id IS NOT OLD.id OR NEW.rakenskapsar_id IS NOT OLD.rakenskapsar_id
       OR NEW.start IS NOT OLD.start OR NEW.slut IS NOT OLD.slut;
    SELECT RAISE(ABORT, 'perioder: en låst period kan inte ändras')
    WHERE OLD.status = 'last';
    SELECT RAISE(ABORT, 'perioder: en stängd period kan inte öppnas igen')
    WHERE OLD.status = 'stangd' AND NEW.status IN ('oppen', 'under_avstamning');
    SELECT RAISE(ABORT, 'perioder: otillåten statusövergång')
    WHERE NOT ((OLD.status = 'oppen' AND NEW.status = 'under_avstamning')
            OR (OLD.status = 'under_avstamning' AND NEW.status IN ('oppen', 'stangd'))
            OR (OLD.status = 'stangd' AND NEW.status = 'last'));
END;
CREATE TRIGGER perioder_ingen_radering BEFORE DELETE ON perioder
BEGIN
    SELECT RAISE(ABORT, 'perioder: en period kan inte raderas');
END;

-- ------------------------------------------------------------- nummerserier
-- Räknare per serie och räkenskapsår. Räknas bara upp, ett steg i taget.
CREATE TABLE nummerserier (
    rakenskapsar_id INTEGER NOT NULL REFERENCES rakenskapsar(id),
    serie           TEXT NOT NULL CHECK (serie IN ('A', 'B', 'I', 'R')),
    nasta           INTEGER NOT NULL CHECK (nasta >= 1),
    PRIMARY KEY (rakenskapsar_id, serie)
) STRICT;

CREATE TRIGGER nummerserier_ny BEFORE INSERT ON nummerserier
BEGIN
    SELECT RAISE(ABORT, 'nummerserier: serien finns redan')
    WHERE EXISTS (SELECT 1 FROM nummerserier
                  WHERE rakenskapsar_id = NEW.rakenskapsar_id AND serie = NEW.serie);
    SELECT RAISE(ABORT, 'nummerserier: en ny serie börjar på 1')
    WHERE NEW.nasta IS NOT 1;
END;
CREATE TRIGGER nummerserier_raknas_upp BEFORE UPDATE ON nummerserier
BEGIN
    SELECT RAISE(ABORT, 'nummerserier: räknaren kan bara räknas upp med ett')
    WHERE NEW.rakenskapsar_id IS NOT OLD.rakenskapsar_id OR NEW.serie IS NOT OLD.serie
       OR NEW.nasta IS NOT OLD.nasta + 1;
END;
CREATE TRIGGER nummerserier_ingen_radering BEFORE DELETE ON nummerserier
BEGIN
    SELECT RAISE(ABORT, 'nummerserier: en räknare kan inte raderas');
END;

-- ---------------------------------------------------------------- kontoplan
-- Versionerad med giltighetsdatum. Ändras bara som signerad systemändring:
-- `andring` pekar på händelsen som bär signaturen.
CREATE TABLE kontoplan (
    id          INTEGER PRIMARY KEY,
    konto       INTEGER NOT NULL CHECK (konto BETWEEN 1000 AND 9999),
    namn        TEXT NOT NULL CHECK (length(trim(namn)) > 0),
    aktiv       INTEGER NOT NULL CHECK (aktiv IN (0, 1)),
    sru         TEXT,
    momskod     TEXT,
    giltig_fran TEXT NOT NULL CHECK (date(giltig_fran) IS giltig_fran),
    andring     INTEGER NOT NULL REFERENCES handelser(seq) DEFERRABLE INITIALLY DEFERRED,
    UNIQUE (konto, giltig_fran)
) STRICT;

CREATE TRIGGER kontoplan_ny BEFORE INSERT ON kontoplan
BEGIN
    SELECT RAISE(ABORT, 'kontoplan: raden finns redan och kan inte ersättas')
    WHERE (EXISTS (SELECT 1 FROM kontoplan WHERE id = NEW.id))
       OR EXISTS (SELECT 1 FROM kontoplan WHERE konto = NEW.konto AND giltig_fran = NEW.giltig_fran);
END;
CREATE TRIGGER kontoplan_ingen_andring BEFORE UPDATE ON kontoplan
BEGIN
    SELECT RAISE(ABORT, 'kontoplan: ändras bara genom en ny signerad version');
END;
CREATE TRIGGER kontoplan_ingen_radering BEFORE DELETE ON kontoplan
BEGIN
    SELECT RAISE(ABORT, 'kontoplan: ändras bara genom en ny signerad version');
END;

-- ------------------------------------------------------------------- regler
-- Konterings-, moms- och kursregler, versionerade med giltighetsdatum.
-- Används från milstolpe 3; tabellen finns redan så att schemat är komplett.
CREATE TABLE regler (
    id          INTEGER PRIMARY KEY,
    namn        TEXT NOT NULL CHECK (length(trim(namn)) > 0),
    version     INTEGER NOT NULL CHECK (version >= 1),
    giltig_fran TEXT NOT NULL CHECK (date(giltig_fran) IS giltig_fran),
    giltig_till TEXT CHECK (giltig_till IS NULL OR date(giltig_till) IS giltig_till),
    innehall    TEXT NOT NULL,
    andring     INTEGER NOT NULL REFERENCES handelser(seq) DEFERRABLE INITIALLY DEFERRED,
    UNIQUE (namn, version)
) STRICT;

CREATE TRIGGER regler_ny BEFORE INSERT ON regler
BEGIN
    SELECT RAISE(ABORT, 'regler: raden finns redan och kan inte ersättas')
    WHERE (EXISTS (SELECT 1 FROM regler WHERE id = NEW.id))
       OR EXISTS (SELECT 1 FROM regler WHERE namn = NEW.namn AND version = NEW.version);
END;
CREATE TRIGGER regler_ingen_andring BEFORE UPDATE ON regler
BEGIN
    SELECT RAISE(ABORT, 'regler: ändras bara genom en ny signerad version');
END;
CREATE TRIGGER regler_ingen_radering BEFORE DELETE ON regler
BEGIN
    SELECT RAISE(ABORT, 'regler: ändras bara genom en ny signerad version');
END;

-- ---------------------------------------------------------------- motparter
-- Motpartsregistret är grunddata och får ändras; ändringen loggas med gamla
-- och nya värden. Verifikationen bär en kopia från bokföringstillfället.
CREATE TABLE motparter (
    id          INTEGER PRIMARY KEY,
    namn        TEXT NOT NULL CHECK (length(trim(namn)) > 0),
    orgnr       TEXT,
    momsnr      TEXT,
    registrerad TEXT NOT NULL,
    andrad      TEXT
) STRICT;

CREATE TRIGGER motparter_ingen_radering BEFORE DELETE ON motparter
BEGIN
    SELECT RAISE(ABORT, 'motparter: en motpart kan inte raderas');
END;

-- ----------------------------------------------------------------- underlag
-- Varje underlag sparas byte för byte; filen ligger i ett arkiv adresserat
-- efter sin SHA-256.
CREATE TABLE underlag (
    id           INTEGER PRIMARY KEY,
    sha256       TEXT NOT NULL UNIQUE CHECK (length(sha256) = 64),
    storlek      INTEGER NOT NULL CHECK (storlek >= 0),
    mottagen     TEXT NOT NULL,
    kanal        TEXT NOT NULL CHECK (kanal IN ('manuell', 'mapp', 'mejl', 'bank')),
    avsandare    TEXT,
    originalnamn TEXT,
    filtyp       TEXT
) STRICT;

CREATE TRIGGER underlag_ny BEFORE INSERT ON underlag
BEGIN
    SELECT RAISE(ABORT, 'underlag: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM underlag WHERE id = NEW.id);
    SELECT RAISE(ABORT, 'underlag: filen är redan mottagen')
    WHERE EXISTS (SELECT 1 FROM underlag WHERE sha256 = NEW.sha256);
END;
CREATE TRIGGER underlag_ingen_andring BEFORE UPDATE ON underlag
BEGIN
    SELECT RAISE(ABORT, 'underlag: ett mottaget underlag kan inte ändras');
END;
CREATE TRIGGER underlag_ingen_radering BEFORE DELETE ON underlag
BEGIN
    SELECT RAISE(ABORT, 'underlag: ett mottaget underlag kan inte raderas');
END;

-- ------------------------------------------------------------ verifikationer
-- Bokförda verifikationer. id är registreringsordningen.
CREATE TABLE verifikationer (
    id                  INTEGER PRIMARY KEY,
    rakenskapsar_id     INTEGER NOT NULL REFERENCES rakenskapsar(id),
    serie               TEXT NOT NULL CHECK (serie IN ('A', 'B', 'I', 'R')),
    nummer              INTEGER NOT NULL CHECK (nummer >= 1),
    period_id           INTEGER NOT NULL REFERENCES perioder(id),
    datum               TEXT NOT NULL CHECK (date(datum) IS datum),
    sammanstallningsdag TEXT NOT NULL CHECK (date(sammanstallningsdag) IS sammanstallningsdag),
    registrerad         TEXT NOT NULL CHECK (registrerad GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T*Z'),
    beskrivning         TEXT NOT NULL CHECK (length(trim(beskrivning)) > 0),
    motpart_namn        TEXT,
    motpart_orgnr       TEXT,
    motpart_momsnr      TEXT,
    forslag_id          INTEGER NOT NULL UNIQUE REFERENCES forslag(id),
    antal_rader         INTEGER NOT NULL CHECK (antal_rader >= 2),
    kanonisk_hash       TEXT NOT NULL CHECK (length(kanonisk_hash) = 64),
    signatur            TEXT NOT NULL,
    nyckel_id           INTEGER NOT NULL REFERENCES nycklar(id),
    kedjehash           TEXT NOT NULL CHECK (length(kedjehash) = 64),
    UNIQUE (rakenskapsar_id, serie, nummer)
) STRICT;

CREATE TRIGGER verifikationer_ny BEFORE INSERT ON verifikationer
BEGIN
    SELECT RAISE(ABORT, 'verifikationer: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM verifikationer WHERE id = NEW.id);
    SELECT RAISE(ABORT, 'verifikationer: perioden tar inte emot poster')
    WHERE (SELECT status FROM perioder WHERE id = NEW.period_id) IS NOT 'oppen';
    SELECT RAISE(ABORT, 'verifikationer: perioden hör till ett annat räkenskapsår')
    WHERE (SELECT rakenskapsar_id FROM perioder WHERE id = NEW.period_id) IS NOT NEW.rakenskapsar_id;
    SELECT RAISE(ABORT, 'verifikationer: datumet ligger utanför perioden')
    WHERE NEW.datum < (SELECT start FROM perioder WHERE id = NEW.period_id)
       OR NEW.datum > (SELECT slut FROM perioder WHERE id = NEW.period_id);
    SELECT RAISE(ABORT, 'verifikationer: numret måste vara nästa i serien')
    WHERE NEW.nummer IS NOT (SELECT COALESCE(MAX(nummer), 0) + 1 FROM verifikationer
                             WHERE rakenskapsar_id = NEW.rakenskapsar_id AND serie = NEW.serie);
    SELECT RAISE(ABORT, 'verifikationer: numret måste tas ur räknaren i samma transaktion')
    WHERE NEW.nummer IS NOT (SELECT nasta - 1 FROM nummerserier
                             WHERE rakenskapsar_id = NEW.rakenskapsar_id AND serie = NEW.serie);
    SELECT RAISE(ABORT, 'verifikationer: förslaget är redan bokfört')
    WHERE EXISTS (SELECT 1 FROM verifikationer WHERE forslag_id = NEW.forslag_id);
END;
CREATE TRIGGER verifikationer_ingen_andring BEFORE UPDATE ON verifikationer
BEGIN
    SELECT RAISE(ABORT, 'verifikationer: det bokförda kan inte ändras; rätta med en ny verifikation i serie R');
END;
CREATE TRIGGER verifikationer_ingen_radering BEFORE DELETE ON verifikationer
BEGIN
    SELECT RAISE(ABORT, 'verifikationer: det bokförda kan inte raderas; rätta med en ny verifikation i serie R');
END;

-- -------------------------------------------------------------------- rader
-- Läggs bara till i bokföringstransaktionen: i ordning, högst antal_rader
-- stycken, och verifikationen måste balansera när sista raden skrivs.
CREATE TABLE rader (
    id              INTEGER PRIMARY KEY,
    verifikation_id INTEGER NOT NULL REFERENCES verifikationer(id),
    radnr           INTEGER NOT NULL CHECK (radnr >= 1),
    konto           INTEGER NOT NULL CHECK (konto BETWEEN 1000 AND 9999),
    debet           INTEGER NOT NULL CHECK (debet >= 0),
    kredit          INTEGER NOT NULL CHECK (kredit >= 0),
    momskod         TEXT,
    text            TEXT,
    CHECK ((debet > 0 AND kredit = 0) OR (kredit > 0 AND debet = 0)),
    UNIQUE (verifikation_id, radnr)
) STRICT;

CREATE TRIGGER rader_ny BEFORE INSERT ON rader
BEGIN
    SELECT RAISE(ABORT, 'rader: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM rader WHERE id = NEW.id);
    SELECT RAISE(ABORT, 'rader: rader läggs bara till i bokföringstransaktionen, i ordning')
    WHERE NEW.radnr IS NOT (SELECT COUNT(*) + 1 FROM rader WHERE verifikation_id = NEW.verifikation_id)
       OR NEW.radnr > (SELECT antal_rader FROM verifikationer WHERE id = NEW.verifikation_id);
    SELECT RAISE(ABORT, 'rader: verifikationen balanserar inte')
    WHERE NEW.radnr = (SELECT antal_rader FROM verifikationer WHERE id = NEW.verifikation_id)
      AND (SELECT COALESCE(SUM(debet - kredit), 0) FROM rader WHERE verifikation_id = NEW.verifikation_id)
          + NEW.debet - NEW.kredit <> 0;
END;
CREATE TRIGGER rader_ingen_andring BEFORE UPDATE ON rader
BEGIN
    SELECT RAISE(ABORT, 'rader: det bokförda kan inte ändras; rätta med en ny verifikation i serie R');
END;
CREATE TRIGGER rader_ingen_radering BEFORE DELETE ON rader
BEGIN
    SELECT RAISE(ABORT, 'rader: det bokförda kan inte raderas; rätta med en ny verifikation i serie R');
END;

-- ---------------------------------------------------------------- rattelser
-- Koppling från original till rättelse, som visas på originalet i alla vyer.
CREATE TABLE rattelser (
    id          INTEGER PRIMARY KEY,
    original_id INTEGER NOT NULL REFERENCES verifikationer(id),
    rattelse_id INTEGER NOT NULL UNIQUE REFERENCES verifikationer(id),
    orsak       TEXT NOT NULL CHECK (length(trim(orsak)) > 0),
    CHECK (original_id <> rattelse_id)
) STRICT;

CREATE TRIGGER rattelser_ny BEFORE INSERT ON rattelser
BEGIN
    SELECT RAISE(ABORT, 'rattelser: raden finns redan och kan inte ersättas')
    WHERE (EXISTS (SELECT 1 FROM rattelser WHERE id = NEW.id))
       OR EXISTS (SELECT 1 FROM rattelser WHERE rattelse_id = NEW.rattelse_id);
    SELECT RAISE(ABORT, 'rattelser: rättelsen ska vara en verifikation i serie R')
    WHERE (SELECT serie FROM verifikationer WHERE id = NEW.rattelse_id) IS NOT 'R';
    SELECT RAISE(ABORT, 'rattelser: rättelsen måste bokföras efter originalet')
    WHERE NEW.rattelse_id <= NEW.original_id;
END;
CREATE TRIGGER rattelser_ingen_andring BEFORE UPDATE ON rattelser
BEGIN
    SELECT RAISE(ABORT, 'rattelser: kopplingen kan bara växa');
END;
CREATE TRIGGER rattelser_ingen_radering BEFORE DELETE ON rattelser
BEGIN
    SELECT RAISE(ABORT, 'rattelser: kopplingen kan bara växa');
END;

-- ------------------------------------------------------------------ forslag
-- Ett förslag har inget verifikationsnummer och ingen period. Det bär sin
-- härkomst och ändras aldrig; beslutet ligger i `beslut`.
CREATE TABLE forslag (
    id               INTEGER PRIMARY KEY,
    serie            TEXT NOT NULL CHECK (serie IN ('A', 'B', 'I', 'R')),
    datum            TEXT NOT NULL CHECK (date(datum) IS datum),
    beskrivning      TEXT NOT NULL CHECK (length(trim(beskrivning)) > 0),
    motpart_id       INTEGER REFERENCES motparter(id),
    motpart_namn     TEXT,
    motpart_orgnr    TEXT,
    motpart_momsnr   TEXT,
    rader            TEXT NOT NULL,
    underlag         TEXT NOT NULL DEFAULT '[]',
    rattar           INTEGER REFERENCES verifikationer(id),
    rattelseorsak    TEXT,
    kalla            TEXT NOT NULL CHECK (kalla IN ('manniska', 'agent', 'system')),
    aktor            TEXT NOT NULL,
    fragor           TEXT,
    flaggor          TEXT,
    sakerhet         REAL CHECK (sakerhet IS NULL OR (sakerhet >= 0 AND sakerhet <= 1)),
    modell           TEXT,
    leverantor       TEXT,
    region           TEXT,
    prompt_hash      TEXT,
    regelversion     TEXT,
    ratt_svar        TEXT,
    idempotensnyckel TEXT UNIQUE,
    skapad           TEXT NOT NULL,
    CHECK ((serie = 'R') = (rattar IS NOT NULL)),
    CHECK ((rattar IS NULL) = (rattelseorsak IS NULL)),
    CHECK (motpart_id IS NULL OR motpart_namn IS NULL)
) STRICT;

CREATE TRIGGER forslag_ny BEFORE INSERT ON forslag
BEGIN
    SELECT RAISE(ABORT, 'forslag: raden finns redan och kan inte ersättas')
    WHERE (EXISTS (SELECT 1 FROM forslag WHERE id = NEW.id))
       OR (NEW.idempotensnyckel IS NOT NULL
           AND EXISTS (SELECT 1 FROM forslag WHERE idempotensnyckel = NEW.idempotensnyckel));
END;
CREATE TRIGGER forslag_ingen_andring BEFORE UPDATE ON forslag
BEGIN
    SELECT RAISE(ABORT, 'forslag: ett förslag ändras inte; beslutet sparas för sig');
END;
CREATE TRIGGER forslag_ingen_radering BEFORE DELETE ON forslag
BEGIN
    SELECT RAISE(ABORT, 'forslag: ett förslag kan inte raderas; avvisa det med en orsak');
END;

-- ------------------------------------------------------------------- beslut
-- Godkännande eller avvisning: vem, när, signatur, orsak och ändringar mot
-- förslaget. Ett förslag får ett beslut, aldrig två.
CREATE TABLE beslut (
    id              INTEGER PRIMARY KEY,
    forslag_id      INTEGER NOT NULL UNIQUE REFERENCES forslag(id),
    utfall          TEXT NOT NULL CHECK (utfall IN ('godkant', 'avvisat')),
    aktor           TEXT NOT NULL,
    tid             TEXT NOT NULL,
    orsak           TEXT,
    andringar       TEXT,
    verifikation_id INTEGER UNIQUE REFERENCES verifikationer(id),
    signatur        TEXT,
    nyckel_id       INTEGER REFERENCES nycklar(id),
    CHECK ((utfall = 'godkant' AND verifikation_id IS NOT NULL AND signatur IS NOT NULL AND nyckel_id IS NOT NULL)
        OR (utfall = 'avvisat' AND verifikation_id IS NULL AND orsak IS NOT NULL AND length(trim(orsak)) > 0))
) STRICT;

CREATE TRIGGER beslut_ny BEFORE INSERT ON beslut
BEGIN
    SELECT RAISE(ABORT, 'beslut: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM beslut WHERE id = NEW.id);
    SELECT RAISE(ABORT, 'beslut: förslaget har redan ett beslut')
    WHERE EXISTS (SELECT 1 FROM beslut WHERE forslag_id = NEW.forslag_id)
       OR (NEW.verifikation_id IS NOT NULL
           AND EXISTS (SELECT 1 FROM beslut WHERE verifikation_id = NEW.verifikation_id));
    SELECT RAISE(ABORT, 'beslut: verifikationen hör inte till förslaget')
    WHERE NEW.utfall = 'godkant'
      AND (SELECT forslag_id FROM verifikationer WHERE id = NEW.verifikation_id) IS NOT NEW.forslag_id;
END;
CREATE TRIGGER beslut_ingen_andring BEFORE UPDATE ON beslut
BEGIN
    SELECT RAISE(ABORT, 'beslut: ett beslut kan inte ändras');
END;
CREATE TRIGGER beslut_ingen_radering BEFORE DELETE ON beslut
BEGIN
    SELECT RAISE(ABORT, 'beslut: ett beslut kan inte raderas');
END;

-- ---------------------------------------------------------- underlag_status
-- Händelser per underlag, till exempel "bokförd som A12".
CREATE TABLE underlag_status (
    id              INTEGER PRIMARY KEY,
    underlag_id     INTEGER NOT NULL REFERENCES underlag(id),
    status          TEXT NOT NULL CHECK (status IN
                    ('mottagen', 'bokford', 'komplettering', 'ej_rakenskapsinformation', 'dubblett')),
    verifikation_id INTEGER REFERENCES verifikationer(id),
    text            TEXT,
    tid             TEXT NOT NULL,
    handelse        INTEGER NOT NULL REFERENCES handelser(seq) DEFERRABLE INITIALLY DEFERRED,
    CHECK ((status IN ('bokford', 'komplettering')) = (verifikation_id IS NOT NULL))
) STRICT;

CREATE TRIGGER underlag_status_ny BEFORE INSERT ON underlag_status
BEGIN
    SELECT RAISE(ABORT, 'underlag_status: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM underlag_status WHERE id = NEW.id);
END;
CREATE TRIGGER underlag_status_ingen_andring BEFORE UPDATE ON underlag_status
BEGIN
    SELECT RAISE(ABORT, 'underlag_status: händelserna kan bara växa');
END;
CREATE TRIGGER underlag_status_ingen_radering BEFORE DELETE ON underlag_status
BEGIN
    SELECT RAISE(ABORT, 'underlag_status: händelserna kan bara växa');
END;

CREATE INDEX rader_konto ON rader (konto);
CREATE INDEX verifikationer_ar ON verifikationer (rakenskapsar_id, serie, nummer);
CREATE INDEX underlag_status_underlag ON underlag_status (underlag_id);
CREATE INDEX underlag_status_verifikation ON underlag_status (verifikation_id);
CREATE INDEX rattelser_original ON rattelser (original_id);
CREATE INDEX handelser_objekt ON handelser (objekt);
