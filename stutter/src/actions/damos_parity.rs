//! What `damo` does and what Stutter does about it (package W11c).
//!
//! A manifest, not an implementation: each `damo` capability is either implemented as a typed,
//! gated Stutter operation, implemented read-only, or deliberately not provided with the reason.
//! A test keeps the manifest honest (no duplicate or empty entry, every implemented claim names a
//! real module path), so parity is stated, not assumed.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parity {
    /// A typed, gated operation with the same effect.
    Operation,
    /// Observed read-only; Stutter never changes it.
    ReadOnly,
    /// Deliberately not provided.
    NotProvided,
}

#[derive(Clone, Copy, Debug)]
pub struct DamoCapability {
    pub damo: &'static str,
    pub parity: Parity,
    /// The Stutter item that covers it, or why it is not provided.
    pub stutter: &'static str,
}

pub const DAMO_PARITY: &[DamoCapability] = &[
    DamoCapability {
        damo: "damo start / stop (kdamond lifecycle)",
        parity: Parity::Operation,
        stutter: "crate::actions::damos_scheme::prepare + DamosCommitted::restore",
    },
    DamoCapability {
        damo: "damo args (vaddr context for one pid)",
        parity: Parity::Operation,
        stutter: "crate::actions::damos_scheme::DamosRequest",
    },
    DamoCapability {
        damo: "damo schemes: stat, pageout, lru_prio, lru_deprio",
        parity: Parity::Operation,
        stutter: "crate::actions::damos_scheme::DamosAction",
    },
    DamoCapability {
        damo: "damo schemes: quotas and size bounds",
        parity: Parity::Operation,
        stutter: "crate::actions::damos_scheme::DamosQuota",
    },
    DamoCapability {
        damo: "damo schemes: quota goals (user_input, some_mem_psi_us)",
        parity: Parity::Operation,
        stutter: "crate::actions::damos_scheme::feed_goal",
    },
    DamoCapability {
        damo: "damo report heats / wss (hot and cold regions)",
        parity: Parity::ReadOnly,
        stutter: "crate::actions::damos_regions::summarize",
    },
    DamoCapability {
        damo: "damo status (read kdamond state)",
        parity: Parity::ReadOnly,
        stutter: "the damon_state sensor",
    },
    DamoCapability {
        damo: "damo schemes: filters, watermarks, stats",
        parity: Parity::NotProvided,
        stutter: "no safe bound to give an unreviewed filter or watermark; use damo directly",
    },
    DamoCapability {
        damo: "damo paddr and fvaddr operation sets",
        parity: Parity::NotProvided,
        stutter: "system-wide physical reclaim is outside the one-process scope",
    },
    DamoCapability {
        damo: "damo record / replay (raw access traces)",
        parity: Parity::NotProvided,
        stutter: "unbounded trace volume; summaries only",
    },
    DamoCapability {
        damo: "taking over a kdamond another owner configured",
        parity: Parity::NotProvided,
        stutter: "never: only an idle kdamond is claimed (module-owned instances are left alone)",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_has_no_duplicate_or_empty_entry_and_names_every_class() {
        let mut seen = std::collections::BTreeSet::new();
        for entry in DAMO_PARITY {
            assert!(!entry.damo.is_empty() && !entry.stutter.is_empty());
            assert!(seen.insert(entry.damo), "duplicate {}", entry.damo);
        }
        for class in [Parity::Operation, Parity::ReadOnly, Parity::NotProvided] {
            assert!(DAMO_PARITY.iter().any(|entry| entry.parity == class));
        }
    }

    #[test]
    fn every_operation_claim_points_at_code_that_exists() {
        let source = include_str!("damos_scheme.rs");
        let regions = include_str!("damos_regions.rs");
        for entry in DAMO_PARITY
            .iter()
            .filter(|e| e.parity != Parity::NotProvided)
        {
            let Some(path) = entry.stutter.strip_prefix("crate::actions::") else {
                continue;
            };
            let (module, rest) = path.split_once("::").unwrap();
            let text = match module {
                "damos_scheme" => source,
                "damos_regions" => regions,
                other => panic!("unexpected module {other}"),
            };
            // The last identifier of the claim must be declared there.
            let symbol = rest
                .split(['+', ' '])
                .find(|part| !part.is_empty())
                .and_then(|part| part.rsplit("::").next())
                .unwrap();
            assert!(
                text.contains(&format!("fn {symbol}"))
                    || text.contains(&format!("struct {symbol}"))
                    || text.contains(&format!("enum {symbol}")),
                "{} names {symbol}, which is not declared",
                entry.damo
            );
        }
    }
}
