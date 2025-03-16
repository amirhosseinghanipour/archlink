use clap::{Arg, Command};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::Command as SysCommand;
use std::time::Duration;
use strsim::levenshtein;
use tokio;
use colored::Colorize;
use serde_json;

const VERSION: &str = "0.1.0";

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

#[derive(Debug)]
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
                            format!("Warning: Invalid config file format: {}", e).yellow()
                        );
                    }
                },
                Err(e) => {
                    eprintln!(
                        "{}",
                        format!("Warning: Failed to read config file: {}", e).yellow()
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
            let query = sub_m.get_one::<String>("query").unwrap().trim();
            if query.is_empty() {
                eprintln!("{}", "Error: Query cannot be empty.".red());
                std::process::exit(1);
            }
            search_packages(query, max_results).await?;
        }
        Some(("install", sub_m)) => {
            let package = sub_m.get_one::<String>("package").unwrap().trim();
            if package.is_empty() {
                eprintln!("{}", "Error: Package name cannot be empty.".red());
                std::process::exit(1);
            }
            install_package(package, "unknown")?;
        }
        _ => unreachable!(),
    }

    Ok(())
}

async fn search_packages(query: &str, max_results: usize) -> Result<(), Box<dyn std::error::Error>> {
    let mut official_results = Vec::new();

    match search_arch_website(query).await {
        Ok(results) => official_results.extend(results),
        Err(e) => eprintln!("{}", format!("Warning: Web search failed: {}", e).yellow()),
    }

    if official_results.is_empty() {
        match search_official_repos(query) {
            Ok(results) => official_results.extend(results),
            Err(e) => eprintln!("{}", format!("Warning: pacman search failed: {}", e).yellow()),
        }
    }

    let aur_results = match search_aur(query).await {
        Ok(results) => results,
        Err(e) => {
            eprintln!("{}", format!("Warning: AUR search failed: {}", e).yellow());
            Vec::new()
        }
    };

    let all_results = rank_results(official_results, aur_results, query, max_results);

    if all_results.is_empty() {
        println!(
            "{}",
            format!("No packages found for '{}'. Try refining your query.", query).yellow()
        );
        return Ok(());
    }

    println!(
        "{}",
        format!("Suggestions for '{}':", query).bold().white()
    );
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
        "Enter the number of the package to install (0 to exit): ".bold().white()
    );
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin()
        .lock()
        .read_line(&mut input)
        .map_err(|e| format!("Failed to read input: {}", e))?;
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
        io::stdin()
            .lock()
            .read_line(&mut confirm)
            .map_err(|e| format!("Failed to read confirmation: {}", e))?;
        if confirm.trim().to_lowercase().starts_with('y') {
            install_package(&selected_package.name, selected_package.source)?;
        } else {
            println!("{}", "Installation cancelled.".yellow());
        }
    } else if choice != 0 {
        println!("{}", "Invalid selection. Exiting.".yellow());
    }

    Ok(())
}

async fn search_arch_website(query: &str) -> Result<Vec<Package>, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
    let url = format!(
        "https://archlinux.org/packages/search/json/?q={}",
        urlencoding::encode(query)
    );
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch Arch website data: {}", e))?;
    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse JSON: {}", e))?;

    let mut packages = Vec::new();
    if let Some(results) = json.get("results").and_then(|r| r.as_array()) {
        for pkg in results {
            let name = pkg["pkgname"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let version = format!(
                "{}-{}",
                pkg["pkgver"].as_str().unwrap_or(""),
                pkg["pkgrel"].as_str().unwrap_or("")
            );
            let description = pkg["pkgdesc"]
                .as_str()
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

fn search_official_repos(query: &str) -> Result<Vec<Package>, String> {
    let output = SysCommand::new("pacman")
        .args(["-Ss", query])
        .output()
        .map_err(|e| format!("Failed to run pacman: {}", e))?;
    let output_str = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        return Err(format!("pacman failed with: {}", output_str));
    }

    let mut packages = Vec::new();
    let mut current_pkg: Option<Package> = None;

    for line in output_str.lines() {
        if line.starts_with(" ") {
            if let Some(ref mut pkg) = current_pkg {
                pkg.description.push_str(line.trim());
                pkg.description.push(' ');
            }
        } else if !line.is_empty() {
            if let Some(pkg) = current_pkg.take() {
                packages.push(pkg);
            }
            let parts: Vec<&str> = line.splitn(2, ' ').collect();
            if parts.len() < 2 {
                continue;
            }
            let name_ver: Vec<&str> = parts[0].split('/').collect();
            let name = name_ver.last().unwrap_or(&"unknown").to_string();
            let version_desc: Vec<&str> = parts[1].splitn(2, ' ').collect();
            let version = version_desc[0].to_string();
            let description = version_desc
                .get(1)
                .unwrap_or(&"No description available")
                .to_string();
            current_pkg = Some(Package {
                name,
                version,
                description,
                source: "official",
            });
        }
    }
    if let Some(pkg) = current_pkg {
        packages.push(pkg);
    }
    Ok(packages)
}

async fn search_aur(query: &str) -> Result<Vec<Package>, Box<dyn std::error::Error>> {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let url = format!(
        "https://aur.archlinux.org/rpc/?v=5&type=search&arg={}",
        urlencoding::encode(query)
    );
    let response = client.get(&url).send().await.map_err(|e| {
        format!(
            "Failed to fetch AUR data: {}. Check your network connection.",
            e
        )
    })?;
    let aur_data = response.json::<AurResponse>().await.map_err(|e| {
        format!(
            "Failed to parse AUR response: {}. AUR API may have changed.",
            e
        )
    })?;

    Ok(aur_data
        .results
        .into_iter()
        .map(|pkg| Package {
            name: pkg.name,
            version: pkg.version,
            description: pkg
                .description
                .unwrap_or("No description available".to_string()),
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
    let mut combined = Vec::new();
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
            format!("Trying 'sudo pacman -S {}'... (may prompt for password)", package)
                .bold()
                .white()
        );
        let status = SysCommand::new("sudo")
            .args(["pacman", "-S", package, "--noconfirm"])
            .status()
            .map_err(|e| format!("Failed to run pacman: {}", e))?;
        if status.success() {
            println!(
                "{}",
                format!("Successfully installed '{}' with pacman", package).green()
            );
            return Ok(());
        }
    }

    let helpers = [("yay", vec!["-S"]), ("paru", vec!["-S"])];
    for (helper, args) in helpers {
        if SysCommand::new("which").arg(helper).output().is_ok() {
            attempted.push(helper);
            println!(
                "{}",
                format!("Trying '{} -S {}'... (may prompt for password)", helper, package)
                    .bold()
                    .white()
            );
            let mut cmd_args = args;
            cmd_args.push(package);
            let status = SysCommand::new(helper)
                .args(&cmd_args)
                .status()
                .map_err(|e| format!("Failed to run {}: {}", helper, e))?;
            if status.success() {
                println!(
                    "{}",
                    format!("Successfully installed '{}' with {}", package, helper).green()
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
