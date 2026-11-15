//! Folder compare: the ids and messages of a run (the state machine is Task 5).

use hoprs_core::Result;
use hoprs_remote::{Progress, Summary};

use super::{App, Effect, TabId};

/// One comparison run of a tab. A tab's runs are told apart by this id, so the late
/// results of a cancelled run can be dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CompareRunId(pub u64);

/// What a running comparison reports.
#[derive(Debug, Clone)]
// `Found` is the common message; boxing it would only add an allocation per difference.
#[allow(clippy::large_enum_variant)]
pub enum CompareMsg {
    /// A difference, a count of entries seen, or the limit being reached.
    Progress {
        tab: TabId,
        run: CompareRunId,
        progress: Progress,
    },
    /// The run ended, always last. A run stopped through [`Effect::CompareCancel`] ends
    /// with `Ok` and [`Summary::stopped`] set (the app ignores it by run id); a closed
    /// session or a root that cannot be listed ends with `Err`.
    Finished {
        tab: TabId,
        run: CompareRunId,
        result: Result<Summary>,
    },
}

impl App {
    /// Routes a comparison message. Task 5 replaces this stub with the state machine.
    pub(super) fn compare_msg(&mut self, _msg: CompareMsg) -> Vec<Effect> {
        Vec::new()
    }
}
