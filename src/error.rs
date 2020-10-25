// TODO: Display impls

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PopError {
    Empty,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PushError<T> {
    Full(T),
}
