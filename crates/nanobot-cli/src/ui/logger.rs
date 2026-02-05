pub struct LoggerService;

impl LoggerService {
    pub fn init() -> anyhow::Result<()> {
        tui_logger::init_logger(log::LevelFilter::Info)
            .map_err(|e| anyhow::anyhow!("Failed to init tui-logger: {}", e))?;
        tui_logger::set_default_level(log::LevelFilter::Info);
        Ok(())
    }
}
