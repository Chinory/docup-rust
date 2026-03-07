use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use toml::Value as TomlValue;
use toml::map::Map as TomlMap;

const DEFAULT_SERVER_URL: &str = "http://localhost:8080";

#[derive(Parser, Debug)]
#[command(
    name = "docup",
    author,
    version,
    about = "Mark Down? Doc Up! Rust demo CLI.",
    long_about = None,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Bootstrap .docup.yaml in the current project.
    Init,
    /// Interactive login (prompts for server URL and token).
    Login,
    /// Sync docs from current project config.
    Sync,
    /// Read/write local config in ~/.docup/config.toml.
    Config {
        #[command(subcommand)]
        command: Option<ConfigSubcommand>,
    },
    /// Print version.
    Version,
}

#[derive(Subcommand, Debug)]
enum ConfigSubcommand {
    /// Show config path.
    Path,
    /// Print all config values.
    Show,
    /// Get config value by dotted key.
    Get { key: String },
    /// Set config value by dotted key.
    Set { key: String, value: String },
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct ProjectConfig {
    site: Option<String>,
    #[serde(default)]
    params: BTreeMap<String, String>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init => cmd_init(),
        Commands::Login => cmd_login(),
        Commands::Sync => cmd_sync(),
        Commands::Config { command } => cmd_config(command),
        Commands::Version => {
            println!("docup {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

fn cmd_init() -> Result<()> {
    let path = PathBuf::from(".docup.yaml");
    if path.exists() {
        bail!("{} already exists", path.display());
    }

    let site = prompt_line("Site (host/path_prefix): ")?;
    let site = site.trim();
    if site.is_empty() {
        bail!("site cannot be empty");
    }

    let mut default_params = BTreeMap::new();
    default_params.insert("api_style".to_string(), "responses".to_string());
    default_params.insert("lang".to_string(), "python".to_string());

    let cfg = ProjectConfig {
        site: Some(site.to_string()),
        params: default_params,
    };

    let yaml = serde_yaml::to_string(&cfg).context("serialize .docup.yaml")?;
    fs::write(&path, yaml).with_context(|| format!("write {}", path.display()))?;
    println!("Initialized {}", path.display());
    Ok(())
}

fn cmd_login() -> Result<()> {
    let config = load_config_table()?;
    let default_server = get_dotted_key(&config, "auth.server_url")
        .and_then(as_string)
        .unwrap_or_else(|| DEFAULT_SERVER_URL.to_string());
    let server_input = prompt_line(&format!("Server URL [{}]: ", default_server))?;
    let server = if server_input.trim().is_empty() {
        default_server
    } else {
        server_input.trim().to_string()
    };
    let token = rpassword::prompt_password("Bearer token: ").context("read token from terminal")?;

    if token.trim().is_empty() {
        bail!("token cannot be empty");
    }

    let mut config = config;
    set_dotted_key(&mut config, "auth.server_url", server);
    set_dotted_key(&mut config, "auth.token", token);
    save_config_table(&config)?;

    println!("Login saved to {}", config_path()?.display());
    println!("Interactive login complete.");
    println!("Note: demo mode does not verify token with server yet.");
    Ok(())
}

fn cmd_sync() -> Result<()> {
    let config = load_config_table()?;
    let token = get_dotted_key(&config, "auth.token")
        .and_then(as_string)
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("not logged in. run 'docup login' first"))?;

    let project = env::current_dir()
        .context("cannot resolve current directory")?
        .canonicalize()
        .context("cannot resolve current directory")?;
    let project_cfg = load_project_config(&project)?;

    let site = project_cfg.site.clone().ok_or_else(|| {
        anyhow::anyhow!("site not found. run 'docup init --site <host/path_prefix>' first")
    })?;
    let server = get_dotted_key(&config, "auth.server_url")
        .and_then(as_string)
        .unwrap_or_else(|| DEFAULT_SERVER_URL.to_string());

    println!("DocCup Sync Plan");
    println!("  project: {}", project.display());
    println!("  site:    {site}");
    println!("  server:  {server}");
    println!("  token:   configured ({})", mask_token(&token));
    println!("  params:");
    if project_cfg.params.is_empty() {
        println!("    (none)");
    } else {
        for (k, v) in project_cfg.params {
            println!("    {k}={v}");
        }
    }

    println!("Demo mode: sync execution is not wired to backend yet.");
    Ok(())
}

fn cmd_config(command: Option<ConfigSubcommand>) -> Result<()> {
    match command {
        None | Some(ConfigSubcommand::Show) => {
            let config = load_config_table()?;
            if config.is_empty() {
                println!("No config found at {}", config_path()?.display());
                return Ok(());
            }
            print_flattened_config("", &config);
            Ok(())
        }
        Some(ConfigSubcommand::Path) => {
            println!("{}", config_path()?.display());
            Ok(())
        }
        Some(ConfigSubcommand::Get { key }) => {
            let config = load_config_table()?;
            if let Some(value) = get_dotted_key(&config, &key) {
                println!("{}", value_to_display(value));
                return Ok(());
            }
            bail!("key not found: {key}")
        }
        Some(ConfigSubcommand::Set { key, value }) => {
            let mut config = load_config_table()?;
            set_dotted_key(&mut config, &key, value.clone());
            save_config_table(&config)?;
            println!("{key} = {value}");
            Ok(())
        }
    }
}

fn load_project_config(project_dir: &Path) -> Result<ProjectConfig> {
    let path = project_dir.join(".docup.yaml");
    if !path.exists() {
        return Ok(ProjectConfig::default());
    }
    let content = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let cfg =
        serde_yaml::from_str(&content).with_context(|| format!("parse {}", path.display()))?;
    Ok(cfg)
}

fn config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot resolve home directory")?;
    Ok(home.join(".docup").join("config.toml"))
}

fn load_config_table() -> Result<TomlMap<String, TomlValue>> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(TomlMap::new());
    }
    let content = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let value: TomlValue =
        toml::from_str(&content).with_context(|| format!("parse {}", path.display()))?;
    let table = value
        .as_table()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("config root must be TOML table"))?;
    Ok(table)
}

fn save_config_table(table: &TomlMap<String, TomlValue>) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let content = toml::to_string_pretty(table).context("serialize config.toml")?;
    fs::write(&path, content).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn get_dotted_key<'a>(table: &'a TomlMap<String, TomlValue>, key: &str) -> Option<&'a TomlValue> {
    let mut parts = key.split('.').filter(|s| !s.is_empty());
    let first = parts.next()?;
    let mut current = table.get(first)?;

    for part in parts {
        current = current.as_table()?.get(part)?;
    }
    Some(current)
}

fn set_dotted_key(table: &mut TomlMap<String, TomlValue>, key: &str, value: String) {
    let parts: Vec<&str> = key.split('.').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return;
    }

    let mut current = table;
    for part in &parts[..parts.len() - 1] {
        let entry = current
            .entry((*part).to_string())
            .or_insert_with(|| TomlValue::Table(TomlMap::new()));
        if !entry.is_table() {
            *entry = TomlValue::Table(TomlMap::new());
        }
        current = entry
            .as_table_mut()
            .expect("entry must be table after replacement");
    }
    current.insert(parts[parts.len() - 1].to_string(), TomlValue::String(value));
}

fn print_flattened_config(prefix: &str, table: &TomlMap<String, TomlValue>) {
    for (key, value) in table {
        let full_key = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };

        if let Some(t) = value.as_table() {
            print_flattened_config(&full_key, t);
        } else {
            println!("{full_key} = {}", value_to_display(value));
        }
    }
}

fn value_to_display(v: &TomlValue) -> String {
    match v {
        TomlValue::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn as_string(v: &TomlValue) -> Option<String> {
    v.as_str().map(ToOwned::to_owned)
}

fn mask_token(token: &str) -> String {
    let keep = token.chars().count().min(4);
    let tail: String = token
        .chars()
        .rev()
        .take(keep)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("***{tail}")
}

fn prompt_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush().context("flush stdout")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("read input from terminal")?;
    Ok(input.trim_end_matches(&['\r', '\n'][..]).to_string())
}
