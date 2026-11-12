//! Start-up recovery of the items a previous run left unfinished.

use std::collections::{HashMap, HashSet};

use hoprs_core::Result;

use crate::model::{TransferId, TransferItem, TransferState};
use crate::store::QueueStore;

/// Recovers the items a previous run left unfinished: `Active` and
/// `AwaitingDecision` items become `Paused` (`QueueStore::recover`), and so
/// do `Queued` ones, so that nothing resumes automatically.
///
/// Returns every item of each tree that still has an unfinished item,
/// finished ones included: an expanded directory counts all of its
/// children when it is resumed, and is never listed again. Trees that are
/// entirely finished are not loaded. A store error (a corrupt queue) is
/// returned as is.
pub(super) fn recover(store: &mut QueueStore) -> Result<Vec<TransferItem>> {
    let unfinished = store.recover()?;
    for item in unfinished
        .iter()
        .filter(|i| i.state == TransferState::Queued)
    {
        // Best effort: if this fails the item is still loaded as paused,
        // and simply resumes under the global policy.
        let _ = store.set_state(item.id, &TransferState::Paused);
    }
    let all = store.list_all()?;
    // Parents are inserted before their children, so ids are ascending
    // down every tree.
    let mut root_of: HashMap<TransferId, TransferId> = HashMap::new();
    for item in &all {
        let root = item
            .parent
            .and_then(|p| root_of.get(&p).copied())
            .unwrap_or(item.id);
        root_of.insert(item.id, root);
    }
    let live: HashSet<TransferId> = all
        .iter()
        .filter(|i| !i.state.is_terminal())
        .map(|i| root_of[&i.id])
        .collect();
    Ok(all
        .into_iter()
        .filter(|i| live.contains(&root_of[&i.id]))
        .map(|mut i| {
            if i.state == TransferState::Queued {
                i.state = TransferState::Paused;
            }
            i
        })
        .collect())
}
