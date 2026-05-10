use stutter::{cli::parse_app_command, commands};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    commands::dispatch(parse_app_command()?).await
}
