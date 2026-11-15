-- Bokis AI-first, schema version 3: säkerhetsnivån, det vill säga vilka
-- signaturer bokföringen godtar.
--
--   mjukvara      alla registrerade nycklar, också en nyckelfil på datorn
--   hardvara      bara hårdvarunycklar (ed25519-sk), och nyckeln måste röras
--   hardvara-pin  som hardvara, och nyckelns PIN måste anges
--
-- Nivån bestäms av en människa, och varje beslut är en signerad
-- systemändring. Den som höjer nivån signerar med en nyckel som klarar den
-- nya nivån, den som sänker den med en nyckel som klarar den gamla. Den
-- senaste raden gäller från och med sin händelse. En bokföring från före
-- version 3 saknar rad och godtar då alla registrerade nycklar, som den alltid
-- har gjort.

-- ------------------------------------------------------------ sakerhetsniva
CREATE TABLE sakerhetsniva (
    id        INTEGER PRIMARY KEY,
    niva      TEXT NOT NULL CHECK (niva IN ('mjukvara', 'hardvara', 'hardvara-pin')),
    -- Nivån som gällde innan, NULL för bokföringens första beslut.
    tidigare  TEXT CHECK (tidigare IS NULL OR tidigare IN ('mjukvara', 'hardvara', 'hardvara-pin')),
    beslutad  TEXT NOT NULL,
    handelse  INTEGER NOT NULL UNIQUE REFERENCES handelser(seq) DEFERRABLE INITIALLY DEFERRED
) STRICT;

CREATE TRIGGER sakerhetsniva_ny BEFORE INSERT ON sakerhetsniva
BEGIN
    SELECT RAISE(ABORT, 'sakerhetsniva: raden finns redan och kan inte ersättas')
    WHERE EXISTS (SELECT 1 FROM sakerhetsniva WHERE id = NEW.id)
       OR EXISTS (SELECT 1 FROM sakerhetsniva WHERE handelse = NEW.handelse);
END;
CREATE TRIGGER sakerhetsniva_ingen_andring BEFORE UPDATE ON sakerhetsniva
BEGIN
    SELECT RAISE(ABORT, 'sakerhetsniva: ett beslut om säkerhetsnivån kan inte ändras');
END;
CREATE TRIGGER sakerhetsniva_ingen_radering BEFORE DELETE ON sakerhetsniva
BEGIN
    SELECT RAISE(ABORT, 'sakerhetsniva: ett beslut om säkerhetsnivån kan inte raderas');
END;
