#[tokio::main]
async fn main() -> Result<(), stutter::StutterError> {
    env_logger::init();

    stutter::run_cli().await
}
