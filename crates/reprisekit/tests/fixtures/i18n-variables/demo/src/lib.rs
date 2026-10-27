reprisekit::strings!("demo");

pub fn title() -> String {
    tr!("window-title", name = "a")
}

pub fn size() -> String {
    tr!("size", kb = 1)
}
