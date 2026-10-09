use signal_forge::terminal_selection::{Position, Selection};
#[test]
fn reversed_unicode_selection_copies_across_multiple_viewports() {
    let rows: Vec<_> = (0..2000).map(|i| format!("λ row {i}")).collect();
    let selection = Selection {
        anchor: Some(Position {
            row: 1800,
            column: 3,
        }),
        head: Some(Position { row: 2, column: 2 }),
        dragging: false,
    };
    let copied = selection.copy(rows.len(), |row| rows[row].clone());
    assert!(copied.starts_with("row 2\nλ row 3\n"));
    assert!(copied.ends_with("\nλ r"));
    assert_eq!(copied.lines().count(), 1799);
    assert_eq!(rows[2], "λ row 2");
}
#[test]
fn select_all_empty_and_eviction_keep_only_retained_text() {
    let mut selection = Selection::default();
    selection.select_all(3, 5);
    assert_eq!(
        selection.copy(3, |row| format!("row {row}")),
        "row 0\nrow 1\nrow 2"
    );
    selection.evict(1);
    assert_eq!(
        selection.copy(2, |row| format!("row {}", row + 1)),
        "row 1\nrow 2"
    );
    selection.evict(2);
    assert!(selection.bounds().is_none());
    selection.select_all(0, 0);
    assert_eq!(selection.copy(0, |_| panic!()), "");
}

#[test]
fn tx_insertions_preserve_selected_pending_rx_row() {
    let mut selection = Selection {
        anchor: Some(Position { row: 2, column: 1 }),
        head: Some(Position { row: 2, column: 4 }),
        dragging: false,
    };
    selection.insert_row(2);
    assert_eq!(
        selection.copy(4, |row| if row == 3 {
            "PENDING".into()
        } else {
            "TX".into()
        }),
        "END"
    );
}
