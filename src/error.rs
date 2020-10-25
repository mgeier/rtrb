// TODO: Display impls

/// Error type for `Consumer::pop()`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PopError {
    /// The queue was empty.
    Empty,
}

/// Error type for `Producer::push()`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PushError<T> {
    /// The queue was full.
    Full(T),
}
