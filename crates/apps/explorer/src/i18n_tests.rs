use reprisekit::tr;

#[test]
fn a_message_with_a_blank_line_keeps_it() {
    assert_eq!(
        tr!(
            "favorites-take-favorites-failed-explorer-cannot-save-your-favorites-e",
            e = "disk full"
        ),
        "Explorer cannot save your Favorites.\n\ndisk full"
    );
}

#[test]
fn the_object_count_agrees_in_number_and_stays_ungrouped() {
    assert_eq!(tr!("status-panels-objects", objects = 0), "0 objects");
    assert_eq!(tr!("status-panels-objects", objects = 1), "1 object");
    assert_eq!(tr!("status-panels-objects", objects = 1234), "1234 objects");
}
