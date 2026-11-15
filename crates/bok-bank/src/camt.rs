//! camt.053 (ISO 20022 Bank-to-Customer Statement), version .02 och .08.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::{
    assign_entry_ids, parse_amount_with_decimal, parse_date, BankError, BankRow, Statement,
};

#[derive(Default)]
struct Balance {
    kind: String,
    amount: Option<i64>,
    credit: bool,
}

#[derive(Default)]
struct Entry {
    amount: Option<i64>,
    debit: bool,
    status: String,
    booking_date: Option<String>,
    value_date: Option<String>,
    bank_ref: Option<String>,
    additional_info: Option<String>,
    end_to_end: Option<String>,
    ocr: Option<String>,
    messages: Vec<String>,
    debtor: Option<String>,
    creditor: Option<String>,
}

#[derive(Default)]
struct StatementBuilder {
    account: String,
    other_account: String,
    currency: String,
    opening: Option<i64>,
    closing: Option<i64>,
    rows: Vec<BankRow>,
    bank_refs: Vec<Option<String>>,
}

fn local_name(e: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).to_string()
}

fn ends_with(path: &[String], tail: &[&str]) -> bool {
    path.len() >= tail.len()
        && path[path.len() - tail.len()..]
            .iter()
            .zip(tail)
            .all(|(a, b)| a == b)
}

fn xml_amount(text: &str) -> Result<i64, BankError> {
    // I XML är decimaltecknet alltid punkt och beloppet alltid positivt.
    let amount = parse_amount_with_decimal(text, Some('.'))?;
    if amount < 0 {
        return Err(BankError::InvalidAmount(text.to_string()));
    }
    Ok(amount)
}

fn non_empty(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("NOTPROVIDED") {
        None
    } else {
        Some(t.to_string())
    }
}

/// Läser alla kontoutdrag i ett camt.053-dokument.
///
/// Bara bokförda poster (status `BOOK`, eller ingen status alls) tas med;
/// preliminära poster (`PDNG`, `INFO`) hoppas över.
pub fn parse_camt053(xml: &[u8]) -> Result<Vec<Statement>, BankError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);

    let mut path: Vec<String> = Vec::new();
    let mut statements: Vec<StatementBuilder> = Vec::new();
    let mut current: Option<StatementBuilder> = None;
    let mut balance: Option<Balance> = None;
    let mut entry: Option<Entry> = None;
    let mut buf = Vec::new();

    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| BankError::Xml(e.to_string()))?;
        match event {
            Event::Start(e) => {
                let name = local_name(&e);
                match name.as_str() {
                    "Stmt" => current = Some(StatementBuilder::default()),
                    "Bal" if current.is_some() => balance = Some(Balance::default()),
                    "Ntry" if current.is_some() => entry = Some(Entry::default()),
                    _ => {}
                }
                path.push(name);
            }
            Event::End(e) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                match name.as_str() {
                    "Bal" => {
                        if let (Some(b), Some(s)) = (balance.take(), current.as_mut()) {
                            if let Some(amount) = b.amount {
                                let signed = if b.credit { amount } else { -amount };
                                match b.kind.as_str() {
                                    "OPBD" | "PRCD" => s.opening = s.opening.or(Some(signed)),
                                    "CLBD" => s.closing = Some(signed),
                                    _ => {}
                                }
                            }
                        }
                    }
                    "Ntry" => {
                        if let (Some(e), Some(s)) = (entry.take(), current.as_mut()) {
                            if e.status.is_empty() || e.status == "BOOK" {
                                let (row, bank_ref) = build_row(e, s)?;
                                s.bank_refs.push(bank_ref);
                                s.rows.push(row);
                            }
                        }
                    }
                    "Stmt" => {
                        if let Some(s) = current.take() {
                            statements.push(s);
                        }
                    }
                    _ => {}
                }
                path.pop();
            }
            Event::Text(t) => {
                let text = t
                    .unescape()
                    .map_err(|e| BankError::Xml(e.to_string()))?
                    .to_string();
                handle_text(
                    &path,
                    &text,
                    current.as_mut(),
                    balance.as_mut(),
                    entry.as_mut(),
                );
            }
            Event::CData(t) => {
                let text = String::from_utf8_lossy(&t.into_inner()).to_string();
                handle_text(
                    &path,
                    &text,
                    current.as_mut(),
                    balance.as_mut(),
                    entry.as_mut(),
                );
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    if statements.is_empty() {
        return Err(BankError::NoStatement);
    }

    Ok(statements
        .into_iter()
        .map(|mut s| {
            if s.account.is_empty() {
                s.account = s.other_account.clone();
            }
            for row in &mut s.rows {
                row.account = s.account.clone();
                if row.currency.is_empty() {
                    row.currency = s.currency.clone();
                }
            }
            assign_entry_ids(&mut s.rows, "camt053", &s.bank_refs);
            Statement {
                account: s.account,
                currency: s.currency,
                opening_balance_ore: s.opening,
                closing_balance_ore: s.closing,
                rows: s.rows,
            }
        })
        .collect())
}

fn handle_text(
    path: &[String],
    text: &str,
    statement: Option<&mut StatementBuilder>,
    balance: Option<&mut Balance>,
    entry: Option<&mut Entry>,
) {
    if let Some(e) = entry {
        let in_tx = path.iter().any(|n| n == "TxDtls");
        if ends_with(path, &["Ntry", "Amt"]) {
            e.amount = xml_amount(text).ok();
        } else if ends_with(path, &["Ntry", "CdtDbtInd"]) {
            e.debit = text.trim() == "DBIT";
        } else if ends_with(path, &["Ntry", "Sts"]) || ends_with(path, &["Ntry", "Sts", "Cd"]) {
            e.status = text.trim().to_string();
        } else if ends_with(path, &["Ntry", "BookgDt", "Dt"])
            || ends_with(path, &["Ntry", "BookgDt", "DtTm"])
        {
            e.booking_date = Some(text.trim().to_string());
        } else if ends_with(path, &["Ntry", "ValDt", "Dt"])
            || ends_with(path, &["Ntry", "ValDt", "DtTm"])
        {
            e.value_date = Some(text.trim().to_string());
        } else if ends_with(path, &["Ntry", "AcctSvcrRef"]) {
            e.bank_ref = non_empty(text);
        } else if ends_with(path, &["Ntry", "AddtlNtryInf"]) {
            e.additional_info = non_empty(text);
        } else if in_tx && ends_with(path, &["Refs", "EndToEndId"]) {
            e.end_to_end = e.end_to_end.take().or_else(|| non_empty(text));
        } else if in_tx && ends_with(path, &["CdtrRefInf", "Ref"]) {
            e.ocr = e.ocr.take().or_else(|| non_empty(text));
        } else if in_tx && ends_with(path, &["RmtInf", "Ustrd"]) {
            if let Some(m) = non_empty(text) {
                e.messages.push(m);
            }
        } else if in_tx
            && (ends_with(path, &["RltdPties", "Dbtr", "Nm"])
                || ends_with(path, &["RltdPties", "Dbtr", "Pty", "Nm"]))
        {
            e.debtor = e.debtor.take().or_else(|| non_empty(text));
        } else if in_tx
            && (ends_with(path, &["RltdPties", "Cdtr", "Nm"])
                || ends_with(path, &["RltdPties", "Cdtr", "Pty", "Nm"]))
        {
            e.creditor = e.creditor.take().or_else(|| non_empty(text));
        }
        return;
    }
    if let Some(b) = balance {
        if ends_with(path, &["Bal", "Tp", "CdOrPrtry", "Cd"]) {
            b.kind = text.trim().to_string();
        } else if ends_with(path, &["Bal", "Amt"]) {
            b.amount = xml_amount(text).ok();
        } else if ends_with(path, &["Bal", "CdtDbtInd"]) {
            b.credit = text.trim() == "CRDT";
        }
        return;
    }
    if let Some(s) = statement {
        if ends_with(path, &["Stmt", "Acct", "Id", "IBAN"]) {
            s.account = text.trim().to_string();
        } else if ends_with(path, &["Stmt", "Acct", "Id", "Othr", "Id"]) {
            s.other_account = text.trim().to_string();
        } else if ends_with(path, &["Stmt", "Acct", "Ccy"]) {
            s.currency = text.trim().to_string();
        }
    }
}

fn build_row(e: Entry, s: &StatementBuilder) -> Result<(BankRow, Option<String>), BankError> {
    let amount = e.amount.ok_or(BankError::Missing("belopp"))?;
    let date = e.booking_date.ok_or(BankError::Missing("bokföringsdag"))?;
    let booking_date = parse_date(&date, "%Y-%m-%d")?;
    let value_date = match e.value_date {
        Some(d) => Some(parse_date(&d, "%Y-%m-%d")?),
        None => None,
    };
    let text = if !e.messages.is_empty() {
        e.messages.join(" ")
    } else {
        e.additional_info.clone().unwrap_or_default()
    };
    let counterparty = if e.debit { e.creditor } else { e.debtor };
    let reference = e.ocr.or(e.end_to_end).or_else(|| e.bank_ref.clone());
    Ok((
        BankRow {
            account: s.account.clone(),
            booking_date,
            value_date,
            amount_ore: if e.debit { -amount } else { amount },
            currency: s.currency.clone(),
            text,
            reference,
            counterparty,
            balance_ore: None,
            entry_id: String::new(),
        },
        e.bank_ref,
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const CAMT_02: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt>
    <GrpHdr><MsgId>MSG-1</MsgId><CreDtTm>2026-11-03T06:00:00</CreDtTm></GrpHdr>
    <Stmt>
      <Id>STMT-1</Id>
      <Acct><Id><IBAN>SE4550000000058398257466</IBAN></Id><Ccy>SEK</Ccy></Acct>
      <Bal>
        <Tp><CdOrPrtry><Cd>OPBD</Cd></CdOrPrtry></Tp>
        <Amt Ccy="SEK">1000.00</Amt><CdtDbtInd>CRDT</CdtDbtInd>
        <Dt><Dt>2026-11-01</Dt></Dt>
      </Bal>
      <Bal>
        <Tp><CdOrPrtry><Cd>CLBD</Cd></CdOrPrtry></Tp>
        <Amt Ccy="SEK">1349.50</Amt><CdtDbtInd>CRDT</CdtDbtInd>
        <Dt><Dt>2026-11-02</Dt></Dt>
      </Bal>
      <Ntry>
        <Amt Ccy="SEK">500.00</Amt><CdtDbtInd>CRDT</CdtDbtInd><Sts>BOOK</Sts>
        <BookgDt><Dt>2026-11-01</Dt></BookgDt><ValDt><Dt>2026-11-01</Dt></ValDt>
        <AcctSvcrRef>BANK-0001</AcctSvcrRef>
        <NtryDtls><TxDtls>
          <Refs><EndToEndId>NOTPROVIDED</EndToEndId></Refs>
          <AmtDtls><TxAmt><Amt Ccy="SEK">500.00</Amt></TxAmt></AmtDtls>
          <RltdPties><Dbtr><Nm>Kund AB</Nm></Dbtr></RltdPties>
          <RmtInf><Ustrd>Faktura 1001</Ustrd></RmtInf>
        </TxDtls></NtryDtls>
      </Ntry>
      <Ntry>
        <Amt Ccy="SEK">150.50</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts>BOOK</Sts>
        <BookgDt><Dt>2026-11-02</Dt></BookgDt>
        <AcctSvcrRef>BANK-0002</AcctSvcrRef>
        <NtryDtls><TxDtls>
          <RltdPties><Cdtr><Nm>Telia Sverige AB</Nm></Cdtr></RltdPties>
          <RmtInf><Strd><CdtrRefInf><Ref>123456789</Ref></CdtrRefInf></Strd></RmtInf>
        </TxDtls></NtryDtls>
        <AddtlNtryInf>Bankgiro</AddtlNtryInf>
      </Ntry>
      <Ntry>
        <Amt Ccy="SEK">99.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts>PDNG</Sts>
        <BookgDt><Dt>2026-11-02</Dt></BookgDt>
      </Ntry>
    </Stmt>
  </BkToCstmrStmt>
</Document>"#;

    const CAMT_08: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ns2:Document xmlns:ns2="urn:iso:std:iso:20022:tech:xsd:camt.053.001.08">
 <ns2:BkToCstmrStmt><ns2:Stmt>
  <ns2:Acct><ns2:Id><ns2:Othr><ns2:Id>5401-1234567</ns2:Id></ns2:Othr></ns2:Id><ns2:Ccy>SEK</ns2:Ccy></ns2:Acct>
  <ns2:Ntry>
   <ns2:Amt Ccy="SEK">0.40</ns2:Amt><ns2:CdtDbtInd>DBIT</ns2:CdtDbtInd>
   <ns2:Sts><ns2:Cd>BOOK</ns2:Cd></ns2:Sts>
   <ns2:BookgDt><ns2:DtTm>2026-11-05T10:15:00</ns2:DtTm></ns2:BookgDt>
   <ns2:NtryDtls><ns2:TxDtls>
    <ns2:Refs><ns2:EndToEndId>E2E-77</ns2:EndToEndId></ns2:Refs>
    <ns2:RltdPties><ns2:Cdtr><ns2:Pty><ns2:Nm>Avgift &amp; Co</ns2:Nm></ns2:Pty></ns2:Cdtr></ns2:RltdPties>
    <ns2:RmtInf><ns2:Ustrd>Öresavrundning</ns2:Ustrd><ns2:Ustrd>november</ns2:Ustrd></ns2:RmtInf>
   </ns2:TxDtls></ns2:NtryDtls>
  </ns2:Ntry>
 </ns2:Stmt></ns2:BkToCstmrStmt>
</ns2:Document>"#;

    #[test]
    fn camt_02_reads_booked_entries_and_balances() {
        let statements = parse_camt053(CAMT_02.as_bytes()).unwrap();
        assert_eq!(statements.len(), 1);
        let s = &statements[0];
        assert_eq!(s.account, "SE4550000000058398257466");
        assert_eq!(s.currency, "SEK");
        assert_eq!(s.opening_balance_ore, Some(100_000));
        assert_eq!(s.closing_balance_ore, Some(134_950));
        assert_eq!(s.rows.len(), 2, "den preliminära posten ska inte tas med");
        assert_eq!(s.balances_match(), Some(true));

        let incoming = &s.rows[0];
        assert_eq!(incoming.amount_ore, 50_000);
        assert_eq!(incoming.counterparty.as_deref(), Some("Kund AB"));
        assert_eq!(incoming.text, "Faktura 1001");
        assert_eq!(incoming.reference.as_deref(), Some("BANK-0001"));
        assert_eq!(incoming.booking_date.to_string(), "2026-11-01");

        let outgoing = &s.rows[1];
        assert_eq!(outgoing.amount_ore, -15_050);
        assert_eq!(outgoing.counterparty.as_deref(), Some("Telia Sverige AB"));
        assert_eq!(outgoing.reference.as_deref(), Some("123456789"));
        assert_eq!(outgoing.text, "Bankgiro");
    }

    #[test]
    fn camt_08_with_prefix_and_small_debits() {
        let statements = parse_camt053(CAMT_08.as_bytes()).unwrap();
        let s = &statements[0];
        assert_eq!(s.account, "5401-1234567");
        assert_eq!(s.rows.len(), 1);
        let r = &s.rows[0];
        assert_eq!(
            r.amount_ore, -40,
            "minustecknet får inte tappas på belopp under en krona"
        );
        assert_eq!(r.booking_date.to_string(), "2026-11-05");
        assert_eq!(r.counterparty.as_deref(), Some("Avgift & Co"));
        assert_eq!(r.text, "Öresavrundning november");
        assert_eq!(r.reference.as_deref(), Some("E2E-77"));
        assert_eq!(s.balances_match(), None);
    }

    #[test]
    fn entry_ids_are_stable_and_unique() {
        let a = parse_camt053(CAMT_02.as_bytes()).unwrap();
        let b = parse_camt053(CAMT_02.as_bytes()).unwrap();
        let ids_a: Vec<_> = a[0].rows.iter().map(|r| r.entry_id.clone()).collect();
        let ids_b: Vec<_> = b[0].rows.iter().map(|r| r.entry_id.clone()).collect();
        assert_eq!(ids_a, ids_b);
        assert_ne!(ids_a[0], ids_a[1]);
        assert!(ids_a.iter().all(|i| i.len() == 64));
    }

    #[test]
    fn broken_xml_and_empty_document() {
        assert!(matches!(
            parse_camt053(b"<Document><Stmt>"),
            Err(BankError::Xml(_)) | Err(BankError::NoStatement)
        ));
        assert_eq!(parse_camt053(b"<Document/>"), Err(BankError::NoStatement));
    }

    #[test]
    fn mismatching_balance_is_detected() {
        let wrong = CAMT_02.replace("1349.50", "1349.00");
        let s = parse_camt053(wrong.as_bytes()).unwrap();
        assert_eq!(s[0].balances_match(), Some(false));
    }
}
