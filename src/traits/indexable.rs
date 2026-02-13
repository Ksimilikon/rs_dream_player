use std::path::Path;

pub trait Indexable {
    fn make_index(&self) -> String;
    fn from_index<P: AsRef<Path>>() -> Self;
}
