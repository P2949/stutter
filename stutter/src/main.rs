#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    stutter::run_cli().await
}
