use std::fs;

use super::*;

fn device(root: &std::path::Path, slot: &str, class: &str, irqs: &[u32]) {
    let dir = root.join(slot);
    fs::create_dir_all(dir.join("msi_irqs")).unwrap();
    fs::write(dir.join("class"), format!("{class}\n")).unwrap();
    for irq in irqs {
        fs::write(dir.join("msi_irqs").join(irq.to_string()), "msix\n").unwrap();
    }
}

#[test]
fn an_irq_is_attributed_by_its_msi_file_and_classified_by_the_pci_class_code() {
    let root = std::env::temp_dir().join(format!("stutter-irq-attr-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    device(&root, "0000:01:00.0", "0x010802", &[120, 121, 122]);
    device(&root, "0000:03:00.0", "0x030000", &[130]);
    device(&root, "0000:05:00.0", "0x020000", &[140, 141]);
    device(&root, "0000:00:14.0", "0x0c0330", &[150]);
    let owners = attribute_irqs(&root);
    assert_eq!(owners.len(), 7);
    assert_eq!(
        IrqOwner::of(&owners, 121),
        IrqOwner::Pci {
            slot: "0000:01:00.0".into(),
            class: PciClass::Storage
        }
    );
    assert!(matches!(
        IrqOwner::of(&owners, 130),
        IrqOwner::Pci {
            class: PciClass::Display,
            ..
        }
    ));
    assert!(matches!(
        IrqOwner::of(&owners, 140),
        IrqOwner::Pci {
            class: PciClass::Network,
            ..
        }
    ));
    assert!(matches!(
        IrqOwner::of(&owners, 150),
        IrqOwner::Pci {
            class: PciClass::Usb,
            ..
        }
    ));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_unclaimed_irq_is_unattributed_and_storage_or_unknown_stays_out_of_automatic_paths() {
    let root = std::env::temp_dir().join(format!("stutter-irq-attr2-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    device(&root, "0000:01:00.0", "0x010601", &[10]);
    device(&root, "0000:05:00.0", "0x020000", &[20]);
    // A class file that is not hex is "other", never a guessed class.
    device(&root, "0000:06:00.0", "garbage", &[30]);
    let owners = attribute_irqs(&root);
    assert_eq!(IrqOwner::of(&owners, 999), IrqOwner::Unattributed);
    assert!(IrqOwner::of(&owners, 999).is_storage_or_unknown());
    assert!(IrqOwner::of(&owners, 10).is_storage_or_unknown());
    assert!(!IrqOwner::of(&owners, 20).is_storage_or_unknown());
    assert!(matches!(
        IrqOwner::of(&owners, 30),
        IrqOwner::Pci {
            class: PciClass::Other,
            ..
        }
    ));
    assert!(attribute_irqs(&root.join("absent")).is_empty());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_real_host_pci_tree_reads_without_panicking() {
    for (irq, owner) in attribute_irqs(std::path::Path::new("/sys/bus/pci/devices")) {
        assert!(irq > 0 || matches!(owner, IrqOwner::Pci { .. }));
    }
}
