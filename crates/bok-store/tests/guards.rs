//! Spärrarna testas mot schemat: varje tabell som bara ska kunna växa har en
//! trigger mot ändring och en mot radering, och de stoppar verkligen.

mod common;

use bok_store::APPEND_ONLY_TABLES;
use common::*;

#[test]
fn every_append_only_table_has_its_guards() {
    let b = book();
    assert_eq!(b.store.missing_guards().unwrap(), Vec::<String>::new());
    let guards = b.store.guards().unwrap();
    let ver = guards
        .iter()
        .find(|g| g.table == "verifikationer" && g.event == "UPDATE")
        .unwrap();
    assert!(!ver.messages.is_empty(), "{ver:?}");
}

#[test]
fn the_guards_really_stop_changes() {
    let mut b = book();
    b.post_purchase(d(2026, 3, 16), "Papper", 10_000);
    let raw = b.raw();
    for table in APPEND_ONLY_TABLES {
        let count: i64 = raw
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        if count == 0 {
            continue;
        }
        let first_column: String = raw
            .query_row(
                &format!("SELECT name FROM pragma_table_info('{table}') WHERE pk = 0 LIMIT 1"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            raw.execute(
                &format!("UPDATE {table} SET {first_column} = {first_column}"),
                []
            )
            .is_err(),
            "{table} gick att ändra"
        );
        assert!(
            raw.execute(&format!("DELETE FROM {table}"), []).is_err(),
            "{table} gick att radera"
        );
    }
}
