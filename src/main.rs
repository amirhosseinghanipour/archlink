use clap::{Arg, Command};
use colored::Colorize;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::Command as SysCommand;
use std::time::Duration;
use strsim::levenshtein;

const VERSION: &str = "0.1.1";

#[derive(Serialize, Deserialize, Debug)]
struct AurPackage {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Description")]
    description: Option<String>,
    #[serde(rename = "Version")]
    version: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct AurResponse {
    #[serde(rename = "results")]
    results: Vec<AurPackage>,
}

#[derive(Debug, Clone)]
struct Package {
    name: String,
    version: String,
    description: String,
    source: &'static str,
}

#[derive(Serialize, Deserialize, Debug)]
struct Config {
    max_results: Option<usize>,
}

impl Config {
    fn load() -> Self {
        let config_path = Path::new("/etc/archlink/config.toml");
        if config_path.exists() {
            match fs::read_to_string(config_path) {
                Ok(contents) => match toml::from_str(&contents) {
                    Ok(config) => return config,
                    Err(e) => {
                        eprintln!(
                            "{}",
                            format!("Warning: Invalid config file format: {e}").yellow()
                        );
                    }
                },
                Err(e) => {
                    eprintln!(
                        "{}",
                        format!("Warning: Failed to read config file: {e}").yellow()
                    );
                }
            }
        }
        Config {
            max_results: Some(10),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load();
    let max_results = config.max_results.unwrap_or(10);

    let client = Client::builder().timeout(Duration::from_secs(10)).build()?;

    let matches = Command::new("archlink")
        .version(VERSION)
        .about("ArchLink helps Arch Linux users to find and install packages")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("search")
                .about("Search for packages in official repos and AUR")
                .arg(
                    Arg::new("query")
                        .help("Package name or keyword to search for")
                        .required(true),
                ),
        )
        .subcommand(
            Command::new("install")
                .about("Install a package directly")
                .arg(
                    Arg::new("package")
                        .help("Exact package name to install")
                        .required(true),
                ),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("search", sub_m)) => {
            let query = sub_m
                .get_one::<String>("query")
                .map(|s| s.as_str())
                .unwrap_or_default()
                .trim();
            if query.is_empty() {
                eprintln!("{}", "Error: Query cannot be empty.".red());
                std::process::exit(1);
            }
            search_packages(&client, query, max_results).await?;
        }
        Some(("install", sub_m)) => {
            let package = sub_m
                .get_one::<String>("package")
                .map(|s| s.as_str())
                .unwrap_or_default()
                .trim();
            if package.is_empty() {
                eprintln!("{}", "Error: Package name cannot be empty.".red());
                std::process::exit(1);
            }
            if let Err(e) = install_package(package, "unknown") {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        _ => unreachable!(),
    }

    Ok(())
}

async fn search_packages(
    client: &Client,
    query: &str,
    max_results: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "Searching official repos and AUR...".bold().white());

    let (official_res, aur_res) = tokio::join!(
        search_arch_website(client, query),
        search_aur(client, query)
    );

    let official_results = match official_res {
        Ok(packages) => packages,
        Err(e) => {
            eprintln!(
                "{}",
                format!("Warning: Official repo search failed: {e}").yellow()
            );
            Vec::new()
        }
    };

    let aur_results = match aur_res {
        Ok(packages) => packages,
        Err(e) => {
            eprintln!("{}", format!("Warning: AUR search failed: {e}").yellow());
            Vec::new()
        }
    };

    let all_results = rank_results(official_results, aur_results, query, max_results);

    if all_results.is_empty() {
        println!(
            "{}",
            format!(
                "No packages found for '{query}'. Try refining your query."
            )
            .yellow()
        );
        return Ok(());
    }

    println!("{}", format!("Suggestions for '{query}':").bold().white());
    for (i, pkg) in all_results.iter().enumerate() {
        println!(
            "{}. {:<30} {:<15} - {} [{}]",
            (i + 1).to_string().bold().white(),
            pkg.name.green(),
            pkg.version.blue(),
            pkg.description,
            pkg.source.cyan()
        );
    }

    print!(
        "{}",
        "Enter the number of the package to install (0 to exit): "
            .bold()
            .white()
    );
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().lock().read_line(&mut input)?;
    let choice = input.trim().parse::<usize>().unwrap_or(0);

    if choice > 0 && choice <= all_results.len() {
        let selected_package = &all_results[choice - 1];
        print!(
            "{}",
            format!("Install '{}' (y/N)? ", selected_package.name)
                .bold()
                .white()
        );
        io::stdout().flush()?;
        let mut confirm = String::new();
        io::stdin().lock().read_line(&mut confirm)?;
        if confirm.trim().to_lowercase().starts_with('y') {
            if let Err(e) = install_package(&selected_package.name, selected_package.source) {
                eprintln!("{e}");
                std::process::exit(1);
            }
        } else {
            println!("{}", "Installation cancelled.".yellow());
        }
    } else if choice != 0 {
        println!("{}", "Invalid selection. Exiting.".yellow());
    }

    Ok(())
}

async fn search_arch_website(client: &Client, query: &str) -> Result<Vec<Package>, reqwest::Error> {
    let url = format!(
        "https://archlinux.org/packages/search/json/?q={}",
        urlencoding::encode(query)
    );
    let response = client.get(&url).send().await?;
    let json: serde_json::Value = response.json().await?;

    let mut packages = Vec::new();
    if let Some(results) = json.get("results").and_then(|r| r.as_array()) {
        for pkg in results {
            let name = pkg
                .get("pkgname")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown")
                .to_string();
            let version = format!(
                "{}-{}",
                pkg.get("pkgver").and_then(|v| v.as_str()).unwrap_or(""),
                pkg.get("pkgrel").and_then(|r| r.as_str()).unwrap_or("")
            );
            let description = pkg
                .get("pkgdesc")
                .and_then(|d| d.as_str())
                .unwrap_or("No description available")
                .to_string();
            packages.push(Package {
                name,
                version,
                description,
                source: "official",
            });
        }
    }
    Ok(packages)
}

async fn search_aur(client: &Client, query: &str) -> Result<Vec<Package>, reqwest::Error> {
    let url = format!(
        "https://aur.archlinux.org/rpc/?v=5&type=search&arg={}",
        urlencoding::encode(query)
    );
    let response = client.get(&url).send().await?;
    let aur_data: AurResponse = response.json().await?;

    Ok(aur_data
        .results
        .into_iter()
        .map(|pkg| Package {
            name: pkg.name,
            version: pkg.version,
            description: pkg
                .description
                .unwrap_or_else(|| "No description available".to_string()),
            source: "aur",
        })
        .collect())
}

fn rank_results(
    official: Vec<Package>,
    aur: Vec<Package>,
    query: &str,
    max_results: usize,
) -> Vec<Package> {
    let mut combined: Vec<Package> = Vec::new();
    combined.extend(official);
    combined.extend(aur);

    let query_words: Vec<&str> = query.split_whitespace().collect();
    combined.sort_by(|a, b| {
        let score_a = score_package(a, query, &query_words);
        let score_b = score_package(b, query, &query_words);
        score_b.cmp(&score_a)
    });

    combined.truncate(max_results);
    combined
}

fn score_package(pkg: &Package, query: &str, query_words: &[&str]) -> u32 {
    let name_dist = levenshtein(&pkg.name, query) as u32;
    let mut score = 1000 - name_dist;

    let desc_lower = pkg.description.to_lowercase();
    for word in query_words {
        if desc_lower.contains(&word.to_lowercase()) {
            score += 50;
        }
    }
    score
}

fn install_package(package: &str, source: &str) -> Result<(), String> {
    let mut attempted = Vec::new();

    if source == "official" || source == "unknown" {
        attempted.push("pacman");
        println!(
            "{}",
            format!(
                "Trying 'sudo pacman -S {package}'... (may prompt for password)"
            )
            .bold()
            .white()
        );
        let status = SysCommand::new("sudo")
            .args(["pacman", "-S", package, "--noconfirm"])
            .status()
            .map_err(|e| format!("Failed to run pacman: {e}"))?;
        if status.success() {
            println!(
                "{}",
                format!("Successfully installed '{package}' with pacman").green()
            );
            return Ok(());
        }
    }

    let helpers = [("yay", &["-S"] as &[&str]), ("paru", &["-S"])];
    for (helper, args) in &helpers {
        if is_command_in_path(helper) {
            attempted.push(helper);
            println!(
                "{}",
                format!(
                    "Trying '{} {} {}'... (may prompt for password)",
                    helper,
                    args.join(" "),
                    package
                )
                .bold()
                .white()
            );
            let mut cmd_args = args.to_vec();
            cmd_args.push(package);
            let status = SysCommand::new(helper)
                .args(&cmd_args)
                .status()
                .map_err(|e| format!("Failed to run {helper}: {e}"))?;
            if status.success() {
                println!(
                    "{}",
                    format!("Successfully installed '{package}' with {helper}").green()
                );
                return Ok(());
            }
        }
    }

    Err(format!(
        "{}",
        format!(
            "Failed to install '{}'. Attempted: {}. Install yay/paru or check package name.",
            package,
            attempted.join(", ")
        )
        .red()
    ))
}

fn is_command_in_path(command: &str) -> bool {
    SysCommand::new("which")
        .arg(command)
        .output()
        .is_ok_and(|output| output.status.success())
}
