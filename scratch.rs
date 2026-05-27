use anyhow::Result;

fn main() {
    let err = anyhow::anyhow!("test_error: this is a test");
    println!("anyhow! format: {err:#}");
}
