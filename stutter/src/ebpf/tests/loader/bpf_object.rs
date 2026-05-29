use super::*;

#[test]
fn read_prebuilt_bpf_object_reads_non_empty_file() {
    let dir = temp_dir("prebuilt-bpf");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("stutter.bpf.o");

    fs::write(&path, b"fake-bpf-object").unwrap();

    let bytes = read_prebuilt_bpf_object(&path).unwrap();
    assert_eq!(bytes, b"fake-bpf-object");

    fs::remove_dir_all(dir).ok();
}

#[test]
fn read_prebuilt_bpf_object_rejects_empty_file() {
    let dir = temp_dir("prebuilt-bpf-empty");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("stutter.bpf.o");

    fs::write(&path, b"").unwrap();

    let err = read_prebuilt_bpf_object(&path).unwrap_err();
    assert!(err.to_string().contains("empty"));

    fs::remove_dir_all(dir).ok();
}

#[test]
fn read_prebuilt_bpf_object_rejects_missing_file() {
    let dir = temp_dir("prebuilt-bpf-missing");
    let path = dir.join("nonexistent.bpf.o");

    let err = read_prebuilt_bpf_object(&path).unwrap_err();
    assert!(err.to_string().contains("failed to read"));
}
