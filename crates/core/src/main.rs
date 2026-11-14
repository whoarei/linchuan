use linchuan_core::{App, Options};
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let directory = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("Usage: linchuan-core DATA_DIRECTORY [NAME]"))?;
    let mut options = Options::new(directory.into());
    options.name = args.next();
    options.discovery = std::env::var("LINCHUAN_NO_DISCOVERY").is_err();
    let app = App::start(options).await?;
    // Headless runner is a development utility, not shipped as the end-user application.
    // Keep owner credentials in a local permission-restricted file, never stdout.
    let ready = serde_json::to_vec(&app.bootstrap())?;
    let ready_path = std::env::args().nth(1).unwrap();
    let ready_path = std::path::Path::new(&ready_path).join("ready.json");
    tokio::fs::write(&ready_path, ready).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&ready_path, std::fs::Permissions::from_mode(0o600)).await?;
    }
    println!(
        "Linchuan {} listening on {}",
        linchuan_core::VERSION,
        app.bootstrap().address
    );
    tokio::signal::ctrl_c().await?;
    app.stop();
    Ok(())
}
