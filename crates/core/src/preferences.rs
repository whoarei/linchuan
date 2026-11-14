use crate::App;
use rusqlite::OptionalExtension;

impl App {
    pub async fn close_to_tray(&self) -> anyhow::Result<bool> {
        self.0
            .store
            .run(|connection, _| {
                let value: Option<String> = connection
                    .query_row(
                        "SELECT value FROM app_settings WHERE key='close_to_tray'",
                        [],
                        |row| row.get(0),
                    )
                    .optional()?;
                Ok(value.as_deref() != Some("0"))
            })
            .await
    }

    pub async fn set_close_to_tray(&self, enabled: bool) -> anyhow::Result<()> {
        self.0.store.run(move |connection, _| {
            connection.execute("INSERT INTO app_settings(key,value) VALUES('close_to_tray',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [if enabled {"1"} else {"0"}])?;
            Ok(())
        }).await
    }
}
