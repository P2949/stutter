//! Exact resource ownership receipts (package W5).
//!
//! A process-name scan only *hints* that another controller exists. A receipt names one exact
//! resource, who holds it, where that was learned, what value was observed and until when the
//! statement is good. Mutation is permitted only by a fresh receipt that says the resource is
//! Stutter's or verifiably unmanaged and whose value still matches; everything else, including
//! an expired or missing receipt, fails closed. The Supported operations do not need a receipt
//! because they act on resources Stutter prepares and journals itself; every other family does.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A task attribute that may be tuned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TaskAttribute {
    Affinity,
    Nice,
    IoPriority,
    Uclamp,
}

/// One exact resource. Each variant carries the identity that stops a recycled id or a renumbered
/// device from being mistaken for the original.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ResourceKey {
    Task {
        pid: u32,
        start_ticks: u64,
        attribute: TaskAttribute,
    },
    Cgroup {
        path: String,
        inode: u64,
        attribute: String,
    },
    CpuFreqPolicy {
        policy: u32,
        related_cpus: Vec<u32>,
    },
    GpuDevice {
        pci_address: String,
    },
    Irq {
        number: u32,
        device: String,
    },
    BlockDevice {
        major: u32,
        minor: u32,
    },
    SchedulerInstance {
        generation: u64,
    },
    VmKnob {
        key: String,
        boot_id: String,
    },
}

impl ResourceKey {
    /// A stable text identity, used as the receipt digest input and for logs.
    pub fn render(&self) -> String {
        match self {
            Self::Task {
                pid,
                start_ticks,
                attribute,
            } => {
                format!("task:{pid}:{start_ticks}:{attribute:?}")
            }
            Self::Cgroup {
                path,
                inode,
                attribute,
            } => format!("cgroup:{path}:{inode}:{attribute}"),
            Self::CpuFreqPolicy {
                policy,
                related_cpus,
            } => {
                format!("cpufreq:{policy}:{related_cpus:?}")
            }
            Self::GpuDevice { pci_address } => format!("gpu:{pci_address}"),
            Self::Irq { number, device } => format!("irq:{number}:{device}"),
            Self::BlockDevice { major, minor } => format!("block:{major}:{minor}"),
            Self::SchedulerInstance { generation } => format!("scheduler:{generation}"),
            Self::VmKnob { key, boot_id } => format!("vm:{key}:{boot_id}"),
        }
    }
}

/// Who holds a resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnerClass {
    /// Stutter prepared and journalled the change.
    StutterOwned,
    /// A named external controller, confirmed through an exact interface.
    ExternalConfirmed,
    /// A process or hint suggests an external controller.
    ExternalSuspected,
    /// The user set it explicitly (`taskset`, `nice`, a configured cgroup).
    UserExplicit,
    /// Readable, never writable by Stutter.
    SharedReadOnly,
    /// Verified, through an exact interface, to have no controller.
    UnmanagedVerified,
    /// Not known. Fails closed.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceReceipt {
    pub key: ResourceKey,
    pub owner: OwnerClass,
    /// The interface that supplied this evidence.
    pub source: String,
    pub observed_at_ns: u64,
    pub expires_at_ns: u64,
    /// The canonical value observed when the receipt was made.
    pub observed_value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptDenial {
    NoReceipt,
    Expired,
    /// The receipt is for a different resource than the one asked about.
    WrongResource,
    Owner(OwnerClass),
    /// The value changed since the receipt: someone else wrote it.
    ExternalChange,
}

/// What a recheck of a held resource found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drift {
    None,
    /// The value is no longer the one observed or applied: never restore over it blindly.
    ExternalChange {
        expected: String,
        current: String,
    },
}

impl ResourceReceipt {
    /// Whether a mutation of `key` is permitted at `now_ns` given the value read just now.
    pub fn permits_mutation(
        &self,
        key: &ResourceKey,
        now_ns: u64,
        current_value: &str,
    ) -> Result<(), ReceiptDenial> {
        if &self.key != key {
            return Err(ReceiptDenial::WrongResource);
        }
        if now_ns >= self.expires_at_ns || now_ns < self.observed_at_ns {
            return Err(ReceiptDenial::Expired);
        }
        match self.owner {
            OwnerClass::StutterOwned | OwnerClass::UnmanagedVerified => {}
            other => return Err(ReceiptDenial::Owner(other)),
        }
        if self.observed_value != current_value {
            return Err(ReceiptDenial::ExternalChange);
        }
        Ok(())
    }

    /// Compares the current value with the one this receipt recorded.
    pub fn drift(&self, current_value: &str) -> Drift {
        if self.observed_value == current_value {
            Drift::None
        } else {
            Drift::ExternalChange {
                expected: self.observed_value.clone(),
                current: current_value.to_owned(),
            }
        }
    }
}

/// Receipts by exact resource, bounded, newest wins.
#[derive(Debug, Default)]
pub struct ReceiptBook {
    receipts: BTreeMap<ResourceKey, ResourceReceipt>,
}

/// Receipts kept at once; the oldest is dropped beyond this.
const MAX_RECEIPTS: usize = 4096;

impl ReceiptBook {
    pub fn record(&mut self, receipt: ResourceReceipt) {
        if self.receipts.len() >= MAX_RECEIPTS
            && !self.receipts.contains_key(&receipt.key)
            && let Some(oldest) = self
                .receipts
                .values()
                .min_by_key(|existing| existing.observed_at_ns)
                .map(|existing| existing.key.clone())
        {
            self.receipts.remove(&oldest);
        }
        self.receipts.insert(receipt.key.clone(), receipt);
    }

    /// Permits a mutation only with a fresh matching receipt.
    pub fn permit(
        &self,
        key: &ResourceKey,
        now_ns: u64,
        current_value: &str,
    ) -> Result<(), ReceiptDenial> {
        self.receipts
            .get(key)
            .ok_or(ReceiptDenial::NoReceipt)?
            .permits_mutation(key, now_ns, current_value)
    }

    pub fn len(&self) -> usize {
        self.receipts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.receipts.is_empty()
    }
}

/// A receipt made from the process-name probe alone. A presence hint can only ever make a
/// resource `ExternalSuspected`, and its absence proves nothing, so it is `Unknown`: neither
/// permits a mutation. Only an exact adapter can produce `UnmanagedVerified`.
pub fn receipt_from_hint(
    key: ResourceKey,
    owner_present: bool,
    value: &str,
    now_ns: u64,
    ttl_ns: u64,
) -> ResourceReceipt {
    ResourceReceipt {
        key,
        owner: if owner_present {
            OwnerClass::ExternalSuspected
        } else {
            OwnerClass::Unknown
        },
        source: "process-name hint".to_owned(),
        observed_at_ns: now_ns,
        expires_at_ns: now_ns.saturating_add(ttl_ns),
        observed_value: value.to_owned(),
    }
}

/// Whether an operation changes a resource that needs an exact receipt. The Supported
/// operations (CPU affinity, cgroup placement) act on resources Stutter prepares and journals.
pub fn operation_needs_receipt(
    operation: crate::autotune::operation::MutationOperationKind,
) -> bool {
    super::operation_domain(operation).is_some()
}

/// What a rollback may do with a resource it applied a value to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreDecision {
    /// The resource still holds exactly what Stutter applied and nobody else owns it: put the
    /// original back.
    Restore,
    /// Someone else wrote the resource, or an external owner now holds it. Restoring would
    /// overwrite their state: leave it, record the conflict, and keep diagnosing.
    LeaveExternal { reason: LeaveReason },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaveReason {
    ValueChanged { applied: String, current: String },
    OwnerNow(OwnerClass),
}

/// Compare-before-restore. `applied` is the canonical value Stutter wrote, `current` what the
/// resource holds now, and `owner_now` what an exact interface says about its owner now.
pub fn restore_decision(applied: &str, current: &str, owner_now: OwnerClass) -> RestoreDecision {
    match owner_now {
        OwnerClass::StutterOwned | OwnerClass::UnmanagedVerified => {}
        other => {
            return RestoreDecision::LeaveExternal {
                reason: LeaveReason::OwnerNow(other),
            };
        }
    }
    if applied == current {
        RestoreDecision::Restore
    } else {
        RestoreDecision::LeaveExternal {
            reason: LeaveReason::ValueChanged {
                applied: applied.to_owned(),
                current: current.to_owned(),
            },
        }
    }
}
