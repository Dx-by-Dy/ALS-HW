pub trait Aggregate: Sized {
    type Event;
    type Command;
}