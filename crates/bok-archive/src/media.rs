//! Filtyp för underlag.
//!
//! [`detect`] avgör typen utifrån innehållet och bara i andra hand utifrån
//! filändelsen: ett underlag som heter `kvitto.jpg` men är en PDF ska visas
//! som PDF, och en fil som påstår sig vara en bild men innehåller HTML ska
//! aldrig visas som något annat än text. [`from_name`] används där innehållet
//! inte är läst, till exempel i en lista.

use std::path::Path;

fn extension(file_name: &str) -> String {
    Path::new(file_name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn starts_with_mail_header(data: &[u8]) -> bool {
    const HEADERS: &[&[u8]] = &[
        b"Received:",
        b"From:",
        b"Return-Path:",
        b"MIME-Version:",
        b"Delivered-To:",
        b"Message-ID:",
        b"Date:",
    ];
    HEADERS
        .iter()
        .any(|h| data.len() >= h.len() && data[..h.len()].eq_ignore_ascii_case(h))
}

/// Filtyp utifrån innehållet, med filändelsen som reserv.
pub fn detect(data: &[u8], file_name: &str) -> String {
    let ext = extension(file_name);
    let t = |s: &str| s.to_string();
    if data.starts_with(b"%PDF-") {
        return t("application/pdf");
    }
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return t("image/png");
    }
    if data.starts_with(&[0xff, 0xd8, 0xff]) {
        return t("image/jpeg");
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return t("image/gif");
    }
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return t("image/webp");
    }
    if data.len() >= 12 && &data[4..8] == b"ftyp" {
        let brand = &data[8..12];
        if [b"heic", b"heix", b"mif1", b"msf1", b"hevc"]
            .iter()
            .any(|b| brand == *b)
        {
            return t("image/heic");
        }
    }
    if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
        return t("image/tiff");
    }
    if data.starts_with(b"PK\x03\x04") {
        return match ext.as_str() {
            "docx" => t("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
            "xlsx" => t("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            _ => t("application/zip"),
        };
    }
    let text = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(data);
    let trimmed_start = text
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map(|i| &text[i..])
        .unwrap_or(&[]);
    if trimmed_start.starts_with(b"<?xml") || (ext == "xml" && trimmed_start.starts_with(b"<")) {
        return t("application/xml");
    }
    if ext == "eml" || starts_with_mail_header(text) {
        return t("message/rfc822");
    }
    let is_text = !text.contains(&0) && std::str::from_utf8(text).is_ok();
    match ext.as_str() {
        "csv" if is_text || !text.contains(&0) => return t("text/csv"),
        "json"
            if is_text && (trimmed_start.starts_with(b"{") || trimmed_start.starts_with(b"[")) =>
        {
            return t("application/json")
        }
        "sie" | "se" | "si" => return t("text/plain"),
        _ => {}
    }
    if is_text {
        return t("text/plain");
    }
    t("application/octet-stream")
}

/// Filtyp utifrån filändelsen eller en lagrad ändelse som `pdf`. En lagrad
/// MIME-typ (med `/`) lämnas som den är. `None` om ändelsen är okänd.
pub fn from_name(name_or_extension: &str) -> Option<String> {
    let t = name_or_extension.trim();
    if t.contains('/') {
        return Some(t.to_string());
    }
    let ext = match t.rsplit_once('.') {
        Some((_, e)) => e.to_ascii_lowercase(),
        None => t.to_ascii_lowercase(),
    };
    let known = match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "tif" | "tiff" => "image/tiff",
        "txt" | "sie" | "se" | "si" => "text/plain",
        "csv" => "text/csv",
        "xml" => "application/xml",
        "json" => "application/json",
        "eml" => "message/rfc822",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "zip" => "application/zip",
        _ => return None,
    };
    Some(known.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_wins_over_the_name() {
        assert_eq!(detect(b"%PDF-1.7 ...", "kvitto.jpg"), "application/pdf");
        assert_eq!(
            detect(b"<html><script>alert(1)</script>", "bild.png"),
            "text/plain"
        );
        assert_eq!(
            detect(&[0, 1, 2, 3, 255], "blob.pdf"),
            "application/octet-stream"
        );
    }

    #[test]
    fn names_and_stored_extensions() {
        assert_eq!(from_name("pdf").as_deref(), Some("application/pdf"));
        assert_eq!(from_name("Kvitto.JPG").as_deref(), Some("image/jpeg"));
        assert_eq!(from_name("image/heic").as_deref(), Some("image/heic"));
        assert_eq!(from_name("skanning"), None);
        assert_eq!(from_name("arkiv.tar.gz"), None);
    }
}
