#[tokio::main]
async fn main() -> Result<(), stutter::error::StutterError> {
    env_logger::init();

    stutter::run_cli().await
}
