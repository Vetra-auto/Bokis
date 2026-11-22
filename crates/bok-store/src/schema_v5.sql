-- Bokis AI-first, schemaversion 5: motpartens adress.
--
-- BFN godtar namn och adress som identifiering av motparten när det inte
-- finns något organisationsnummer, till exempel för en privatperson
-- (BFNAR 2013:2, kommentaren till punkt 5.6). Adressen kopieras in på
-- förslaget och verifikationen precis som namn och nummer, så att en
-- bokförd verifikation behåller adressen från bokföringstillfället.
--
-- Kolumnerna läggs till i befintliga tabeller. Deras spärrar från
-- version 1 gäller hela raden: verifikationer och förslag kan inte ändras
-- eller raderas, och en motpart kan inte raderas. Ändringar i
-- motpartsregistret loggas med gamla och nya värden.

ALTER TABLE motparter ADD COLUMN adress TEXT
    CHECK (adress IS NULL OR length(trim(adress)) > 0);
ALTER TABLE forslag ADD COLUMN motpart_adress TEXT
    CHECK (motpart_adress IS NULL OR length(trim(motpart_adress)) > 0);
ALTER TABLE verifikationer ADD COLUMN motpart_adress TEXT
    CHECK (motpart_adress IS NULL OR length(trim(motpart_adress)) > 0);
