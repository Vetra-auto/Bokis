-- Bokis AI-first, schema version 4: inventarieregistret.
--
-- Ett inventarium registreras med anskaffningsvärde, nyttjandeperiod och
-- konton, och skrivs av linjärt enligt plan (BFNAR 2016:10, K2). Registret
-- bokför ingenting: årets avskrivningar och en avyttring blir förslag till
-- verifikationer som en människa granskar och signerar.
--
-- Båda tabellerna kan bara växa. Ett inventarium som har lämnat företaget får
-- en rad i inventarieavgangar: försäljning, utrangering eller, om det var
-- registrerat av misstag, felregistrering med en förklaring. Ett felregistrerat
-- inventarium räknas som om det aldrig hade funnits, och en ny, rätt
-- registrering görs i stället. Ingen rad ändras eller raderas.

-- -------------------------------------------------------------- inventarier
CREATE TABLE inventarier (
    id                 INTEGER PRIMARY KEY,
    beskrivning        TEXT NOT NULL CHECK (length(trim(beskrivning)) > 0),
    anskaffad          TEXT NOT NULL CHECK (date(anskaffad) IS anskaffad),
    -- Öre, utan moms som får dras av.
    anskaffningsvarde  INTEGER NOT NULL CHECK (anskaffningsvarde > 0),
    restvarde          INTEGER NOT NULL CHECK (restvarde >= 0 AND restvarde < anskaffningsvarde),
    -- Månader.
    nyttjandeperiod    INTEGER NOT NULL CHECK (nyttjandeperiod BETWEEN 1 AND 600),
    -- Första dagen i den första månaden som skrivs av.
    avskrivning_fran   TEXT NOT NULL CHECK (
        date(avskrivning_fran) IS avskrivning_fran
        AND avskrivning_fran = date(avskrivning_fran, 'start of month')
        AND avskrivning_fran >= date(anskaffad, 'start of month')),
    tillgangskonto     INTEGER NOT NULL CHECK (tillgangskonto BETWEEN 1000 AND 2999),
    avskrivningskonto  INTEGER NOT NULL CHECK (avskrivningskonto BETWEEN 1000 AND 2999
                                               AND avskrivningskonto <> tillgangskonto),
    kostnadskonto      INTEGER NOT NULL CHECK (kostnadskonto BETWEEN 3000 AND 9999),
    -- Verifikationen som anskaffningen bokfördes på, om den finns i Bokis.
    verifikation_id    INTEGER REFERENCES verifikationer(id),
    aktor              TEXT NOT NULL,
    registrerad        TEXT NOT NULL,
    handelse           INTEGER NOT NULL UNIQUE REFERENCES handelser(seq) DEFERRABLE INITIALLY DEFERRED
) STRICT;

CREATE TRIGGER inventarier_ny BEFORE INSERT ON inventarier
BEGIN
    SELECT RAISE(ABORT, 'inventarier: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM inventarier WHERE id = NEW.id)
       OR EXISTS (SELECT 1 FROM inventarier WHERE handelse = NEW.handelse);
END;
CREATE TRIGGER inventarier_ingen_andring BEFORE UPDATE ON inventarier
BEGIN
    SELECT RAISE(ABORT, 'inventarier: ett registrerat inventarium kan inte ändras; registrera en felregistrering och ett nytt');
END;
CREATE TRIGGER inventarier_ingen_radering BEFORE DELETE ON inventarier
BEGIN
    SELECT RAISE(ABORT, 'inventarier: ett registrerat inventarium kan inte raderas');
END;

-- ------------------------------------------------------- inventarieavgangar
CREATE TABLE inventarieavgangar (
    id             INTEGER PRIMARY KEY,
    inventarie_id  INTEGER NOT NULL UNIQUE REFERENCES inventarier(id),
    datum          TEXT NOT NULL CHECK (date(datum) IS datum),
    typ            TEXT NOT NULL CHECK (typ IN ('forsaljning', 'utrangering', 'felregistrering')),
    -- Öre utan moms; bara en försäljning har ett pris.
    pris           INTEGER NOT NULL CHECK (pris >= 0 AND (typ = 'forsaljning' OR pris = 0)),
    forklaring     TEXT CHECK (forklaring IS NULL OR length(trim(forklaring)) > 0),
    aktor          TEXT NOT NULL,
    registrerad    TEXT NOT NULL,
    handelse       INTEGER NOT NULL UNIQUE REFERENCES handelser(seq) DEFERRABLE INITIALLY DEFERRED,
    CHECK (typ <> 'felregistrering' OR forklaring IS NOT NULL)
) STRICT;

CREATE TRIGGER inventarieavgangar_ny BEFORE INSERT ON inventarieavgangar
BEGIN
    SELECT RAISE(ABORT, 'inventarieavgangar: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM inventarieavgangar WHERE id = NEW.id)
       OR EXISTS (SELECT 1 FROM inventarieavgangar WHERE inventarie_id = NEW.inventarie_id)
       OR EXISTS (SELECT 1 FROM inventarieavgangar WHERE handelse = NEW.handelse);
    SELECT RAISE(ABORT, 'inventarieavgangar: avgången kan inte vara före anskaffningen')
    WHERE NEW.datum < (SELECT anskaffad FROM inventarier WHERE id = NEW.inventarie_id);
END;
CREATE TRIGGER inventarieavgangar_ingen_andring BEFORE UPDATE ON inventarieavgangar
BEGIN
    SELECT RAISE(ABORT, 'inventarieavgangar: en avgång kan inte ändras');
END;
CREATE TRIGGER inventarieavgangar_ingen_radering BEFORE DELETE ON inventarieavgangar
BEGIN
    SELECT RAISE(ABORT, 'inventarieavgangar: en avgång kan inte raderas');
END;
