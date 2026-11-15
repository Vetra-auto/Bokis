-- Bokis AI-first, schema version 2: bankrader och avstämning (steg 4) samt
-- ärenden, det vill säga agentens frågor och flaggor till granskaren.
--
-- Kontoutdragets rader hör till underlaget de kom med och kan bara växa.
-- En bankrad är avstämd när den har en matchning: mot en bokförd
-- verifikation som rör bankens bokföringskonto med exakt radens belopp, eller
-- med en skriftlig förklaring. Bara en människa matchar, och en matchning kan
-- inte ändras eller tas bort. Agenten kan lägga matchningsförslag, som är
-- underlag för granskaren och aldrig avstämmer något själva. En period kan
-- inte stängas så länge en bankrad daterad i perioden saknar matchning
-- (kontrolleras i programmet vid periodövergången).

-- ---------------------------------------------------------------- bankrader
CREATE TABLE bankrader (
    id            INTEGER PRIMARY KEY,
    underlag_id   INTEGER NOT NULL REFERENCES underlag(id),
    -- Stabilt id från kontoutdraget (SHA-256), så att en fil kan läsas in
    -- igen utan att raderna dubbleras.
    post_id       TEXT NOT NULL UNIQUE CHECK (length(post_id) = 64),
    bankkonto     TEXT NOT NULL CHECK (length(trim(bankkonto)) > 0),
    -- Bokföringskontot som banken motsvarar, till exempel 1930.
    konto         INTEGER NOT NULL CHECK (konto BETWEEN 1000 AND 9999),
    bokforingsdag TEXT NOT NULL CHECK (date(bokforingsdag) IS bokforingsdag),
    valutadag     TEXT CHECK (valutadag IS NULL OR date(valutadag) IS valutadag),
    -- Öre. Negativt är pengar ut från kontot.
    belopp        INTEGER NOT NULL,
    valuta        TEXT NOT NULL,
    text          TEXT NOT NULL,
    referens      TEXT,
    motpart       TEXT,
    saldo         INTEGER,
    importerad    TEXT NOT NULL,
    handelse      INTEGER NOT NULL REFERENCES handelser(seq) DEFERRABLE INITIALLY DEFERRED
) STRICT;

CREATE TRIGGER bankrader_ny BEFORE INSERT ON bankrader
BEGIN
    SELECT RAISE(ABORT, 'bankrader: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM bankrader WHERE id = NEW.id)
       OR EXISTS (SELECT 1 FROM bankrader WHERE post_id = NEW.post_id);
END;
CREATE TRIGGER bankrader_ingen_andring BEFORE UPDATE ON bankrader
BEGIN
    SELECT RAISE(ABORT, 'bankrader: en inläst bankrad kan inte ändras');
END;
CREATE TRIGGER bankrader_ingen_radering BEFORE DELETE ON bankrader
BEGIN
    SELECT RAISE(ABORT, 'bankrader: en inläst bankrad kan inte raderas');
END;
CREATE INDEX bankrader_dag ON bankrader (bokforingsdag);

-- ---------------------------------------------------------- bankmatchningar
-- Avstämningen av en bankrad. Exakt ett av verifikation och förklaring.
-- En bankrad avstäms en gång och en verifikation matchas mot högst en rad.
CREATE TABLE bankmatchningar (
    id              INTEGER PRIMARY KEY,
    bankrad_id      INTEGER NOT NULL UNIQUE REFERENCES bankrader(id),
    verifikation_id INTEGER UNIQUE REFERENCES verifikationer(id),
    forklaring      TEXT CHECK (forklaring IS NULL OR length(trim(forklaring)) > 0),
    aktor           TEXT NOT NULL,
    tid             TEXT NOT NULL,
    handelse        INTEGER NOT NULL REFERENCES handelser(seq) DEFERRABLE INITIALLY DEFERRED,
    CHECK ((verifikation_id IS NULL) <> (forklaring IS NULL))
) STRICT;

-- Varje UNIQUE-kolumn kontrolleras, så att INSERT OR REPLACE inte kan ta bort
-- en befintlig avstämning (DELETE-triggers körs inte vid REPLACE).
CREATE TRIGGER bankmatchningar_ny BEFORE INSERT ON bankmatchningar
BEGIN
    SELECT RAISE(ABORT, 'bankmatchningar: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM bankmatchningar WHERE id = NEW.id)
       OR EXISTS (SELECT 1 FROM bankmatchningar WHERE bankrad_id = NEW.bankrad_id)
       OR (NEW.verifikation_id IS NOT NULL
           AND EXISTS (SELECT 1 FROM bankmatchningar WHERE verifikation_id = NEW.verifikation_id));
END;
CREATE TRIGGER bankmatchningar_ingen_andring BEFORE UPDATE ON bankmatchningar
BEGIN
    SELECT RAISE(ABORT, 'bankmatchningar: en avstämning kan inte ändras');
END;
CREATE TRIGGER bankmatchningar_ingen_radering BEFORE DELETE ON bankmatchningar
BEGIN
    SELECT RAISE(ABORT, 'bankmatchningar: en avstämning kan inte raderas');
END;

-- -------------------------------------------------------- matchningsforslag
-- Agentens (eller någon annans) förslag på vad en bankrad motsvarar: en
-- bokförd verifikation, ett verifikationsförslag som väntar på beslut, eller
-- en förklaring. Förslaget avstämmer ingenting; granskaren matchar.
CREATE TABLE matchningsforslag (
    id              INTEGER PRIMARY KEY,
    bankrad_id      INTEGER NOT NULL REFERENCES bankrader(id),
    verifikation_id INTEGER REFERENCES verifikationer(id),
    forslag_id      INTEGER REFERENCES forslag(id),
    forklaring      TEXT CHECK (forklaring IS NULL OR length(trim(forklaring)) > 0),
    kalla           TEXT NOT NULL CHECK (kalla IN ('manniska', 'agent', 'system')),
    aktor           TEXT NOT NULL,
    sakerhet        REAL CHECK (sakerhet IS NULL OR (sakerhet >= 0 AND sakerhet <= 1)),
    modell          TEXT,
    motivering      TEXT,
    skapad          TEXT NOT NULL,
    handelse        INTEGER NOT NULL REFERENCES handelser(seq) DEFERRABLE INITIALLY DEFERRED,
    CHECK ((verifikation_id IS NOT NULL) + (forslag_id IS NOT NULL) + (forklaring IS NOT NULL) = 1)
) STRICT;

CREATE TRIGGER matchningsforslag_ny BEFORE INSERT ON matchningsforslag
BEGIN
    SELECT RAISE(ABORT, 'matchningsforslag: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM matchningsforslag WHERE id = NEW.id);
END;
CREATE TRIGGER matchningsforslag_ingen_andring BEFORE UPDATE ON matchningsforslag
BEGIN
    SELECT RAISE(ABORT, 'matchningsforslag: ett förslag kan inte ändras');
END;
CREATE TRIGGER matchningsforslag_ingen_radering BEFORE DELETE ON matchningsforslag
BEGIN
    SELECT RAISE(ABORT, 'matchningsforslag: ett förslag kan inte raderas');
END;

-- ------------------------------------------------------------------ arenden
-- Frågor och flaggor till granskaren, oftast från agenten. Ett ärende ändrar
-- ingenting i bokföringen. En flagga om att ett underlag är en dubblett eller
-- inte är räkenskapsinformation ändrar underlagets status först när
-- granskaren beslutar det; annars skulle en agent kunna gömma underlag.
CREATE TABLE arenden (
    id          INTEGER PRIMARY KEY,
    typ         TEXT NOT NULL CHECK (typ IN ('fraga', 'flagga')),
    underlag_id INTEGER REFERENCES underlag(id),
    -- För flaggor: 'dubblett', 'ej_rakenskapsinformation', 'behover_komplettering' eller 'olasligt'.
    orsak       TEXT,
    text        TEXT NOT NULL CHECK (length(trim(text)) > 0),
    kalla       TEXT NOT NULL CHECK (kalla IN ('manniska', 'agent', 'system')),
    aktor       TEXT NOT NULL,
    modell      TEXT,
    sakerhet    REAL CHECK (sakerhet IS NULL OR (sakerhet >= 0 AND sakerhet <= 1)),
    skapad      TEXT NOT NULL,
    handelse    INTEGER NOT NULL REFERENCES handelser(seq) DEFERRABLE INITIALLY DEFERRED,
    CHECK (typ = 'fraga' OR (underlag_id IS NOT NULL AND orsak IS NOT NULL))
) STRICT;

CREATE TRIGGER arenden_ny BEFORE INSERT ON arenden
BEGIN
    SELECT RAISE(ABORT, 'arenden: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM arenden WHERE id = NEW.id);
END;
CREATE TRIGGER arenden_ingen_andring BEFORE UPDATE ON arenden
BEGIN
    SELECT RAISE(ABORT, 'arenden: ett ärende kan inte ändras');
END;
CREATE TRIGGER arenden_ingen_radering BEFORE DELETE ON arenden
BEGIN
    SELECT RAISE(ABORT, 'arenden: ett ärende kan inte raderas');
END;

-- Granskarens svar. Ett ärende besvaras en gång. `status` är satt när svaret
-- också ändrade underlagets status.
CREATE TABLE arendesvar (
    id        INTEGER PRIMARY KEY,
    arende_id INTEGER NOT NULL UNIQUE REFERENCES arenden(id),
    svar      TEXT NOT NULL CHECK (length(trim(svar)) > 0),
    status    TEXT CHECK (status IS NULL OR status IN ('ej_rakenskapsinformation', 'dubblett')),
    aktor     TEXT NOT NULL,
    tid       TEXT NOT NULL,
    handelse  INTEGER NOT NULL REFERENCES handelser(seq) DEFERRABLE INITIALLY DEFERRED
) STRICT;

CREATE TRIGGER arendesvar_ny BEFORE INSERT ON arendesvar
BEGIN
    SELECT RAISE(ABORT, 'arendesvar: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM arendesvar WHERE id = NEW.id)
       OR EXISTS (SELECT 1 FROM arendesvar WHERE arende_id = NEW.arende_id);
END;
CREATE TRIGGER arendesvar_ingen_andring BEFORE UPDATE ON arendesvar
BEGIN
    SELECT RAISE(ABORT, 'arendesvar: ett svar kan inte ändras');
END;
CREATE TRIGGER arendesvar_ingen_radering BEFORE DELETE ON arendesvar
BEGIN
    SELECT RAISE(ABORT, 'arendesvar: ett svar kan inte raderas');
END;
