use std::path::PathBuf;

#[derive(Clone, Debug)]
// Some fields are consumed on Linux only (mem_limit_bytes) or by the jobs API
// added in a later task (workers, result_ttl_secs); keep the full config contract.
#[allow(dead_code)]
pub struct Config {
    pub port: u16,
    pub cli_path: PathBuf,
    pub pdfium_dir: Option<String>,
    pub data_dir: PathBuf,
    pub sync_max_bytes: u64,
    pub jobs_max_bytes: u64,
    pub sync_max_pages: u32,
    pub timeout_secs: u64,
    pub output_cap_bytes: u64,
    pub mem_limit_bytes: u64,
    pub workers: usize,
    pub result_ttl_secs: u64,
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

impl Config {
    pub fn from_env() -> Config {
        let default_cli = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("docray")))
            .filter(|p| p.exists())
            .unwrap_or_else(|| PathBuf::from("docray"));
        Config {
            port: env_or("DOCRAY_PORT", 41619),
            cli_path: std::env::var("DOCRAY_CLI_PATH")
                .map(PathBuf::from)
                .unwrap_or(default_cli),
            pdfium_dir: std::env::var("DOCRAY_PDFIUM_DIR").ok(),
            data_dir: std::env::var("DOCRAY_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| "./data".into()),
            sync_max_bytes: env_or("DOCRAY_SYNC_MAX_BYTES", 26_214_400),
            jobs_max_bytes: env_or("DOCRAY_JOBS_MAX_BYTES", 1_073_741_824), // 1 GiB
            sync_max_pages: env_or("DOCRAY_SYNC_MAX_PAGES", 200),
            timeout_secs: env_or("DOCRAY_TIMEOUT_SECS", 300),
            output_cap_bytes: env_or("DOCRAY_OUTPUT_CAP_BYTES", 536_870_912),
            mem_limit_bytes: env_or("DOCRAY_MEM_LIMIT_BYTES", 2_147_483_648),
            // Clamp to at least 1: DOCRAY_WORKERS=0 would spawn no job workers, so
            // submitted jobs would sit 'queued' forever. (Also keeps the sync
            // Semaphore in http.rs non-zero.)
            workers: env_or("DOCRAY_WORKERS", num_cpus::get()).max(1),
            result_ttl_secs: env_or("DOCRAY_RESULT_TTL_SECS", 86_400),
        }
    }
}
