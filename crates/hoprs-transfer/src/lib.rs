mod conflict;
mod model;

pub use conflict::{Decision, decide};
pub use model::{
    ConflictInfo, ConflictPolicy, Direction, TransferId, TransferItem, TransferSpec, TransferState,
};
