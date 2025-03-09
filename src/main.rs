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

#[derive(Serialize, Deserialize, Default)]
struct Config {
    max_results: usize,
}

impl Config {
    fn load() -> Result<Self, String> {
        let config_path = Path::new("/etc/archlink/config.toml");
        if config_path.exists() {
            let contents = fs::read_to_string(config_path)
                .map_err(|e| format!("Failed to read config file: {}", e))?;
            toml::from_str(&contents)
                .map_err(|e| format!("Invalid config file format: {}", e))
        } else {
            Ok(Config { max_results: 10 })
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = match Config::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("{}", format!("Warning: {}", e).yellow());
            Config::default()
        }
    };

    let matches = Command::new("archlink")
        .version(VERSION)
        .about("ArchLink helps Arch Linux users to find the right packages to install")
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
            search_packages(query, &config).await?;
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

async fn search_packages(query: &str, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let official_results = search_official_repos(query)?;
    let aur_results = search_aur(query).await?;
    let all_results = rank_results(official_results, aur_results, query, config.max_results);

    if all_results.is_empty() {
        println!("{}", format!("No packages found for '{}'. Try refining your query.", query).yellow());
        return Ok(());
    }

    println!("{}", format!("Suggestions for '{}':", query).bold());
    for (i, pkg) in all_results.iter().enumerate() {
        println!(
            "{}. {:<30} {:<15} - {} [{}]",
            (i + 1).to_string().bold(),
            pkg.name.green(),
            pkg.version.blue(),
            pkg.description,
            pkg.source.cyan()
        );
    }

    print!("{}", "Enter the number of the package to install (0 to exit): ".bold());
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin()
        .lock()
        .read_line(&mut input)
        .map_err(|e| format!("Failed to read input: {}", e))?;
    let choice = input.trim().parse::<usize>().unwrap_or(0);

    if choice > 0 && choice <= all_results.len() {
        let selected_package = &all_results[choice - 1];
        print!("{}", format!("Install '{}' (y/N)? ", selected_package.name).bold());
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

fn search_official_repos(query: &str) -> Result<Vec<Package>, String> {
    let query_words: Vec<&str> = query.split_whitespace().collect();
    let mut results = Vec::new();

    for word in query_words {
        let output = SysCommand::new("pacman")
            .args(["-Ss", word])
            .output()
            .map_err(|e| {
                format!(
                    "Failed to run 'pacman -Ss {}': {}. Is pacman installed?",
                    word, e
                )
            })?;

        let output_str = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() && output_str.trim().is_empty() {
            continue;
        } else if !output.status.success() {
            return Err(format!(
                "pacman -Ss '{}' failed with exit code: {}. Output: '{}'",
                word, output.status, output_str
            ));
        }

        let mut current_name = None;
        let mut description = String::new();

        for line in output_str.lines() {
            if line.starts_with(" ") {
                if current_name.is_some() {
                    description.push_str(line.trim());
                    description.push(' ');
                }
            } else if !line.is_empty() {
                if let Some(name) = current_name.take() {
                    results.push(Package {
                        name,
                        version: String::new(), 
                        description: if description.is_empty() {
                            "No description available".to_string()
                        } else {
                            description.trim().to_string()
                        },
                        source: "official",
                    });
                }
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 2 {
                    continue;
                }
                current_name = Some(parts[0].to_string());
                let version = parts[1].to_string(); 
                description = parts.get(2..).map(|p| p.join(" ")).unwrap_or_default();
                if let Some(name) = current_name.take() {
                    results.push(Package {
                        name,
                        version,
                        description: if description.is_empty() {
                            "No description available".to_string()
                        } else {
                            description.trim().to_string()
                        },
                        source: "official",
                    });
                }
            }
        }
        
        if let Some(name) = current_name {
            results.push(Package {
                name,
                version: String::new(), 
                description: if description.is_empty() {
                    "No description available".to_string()
                } else {
                    description.trim().to_string()
                },
                source: "official",
            });
        }
    }

    use std::collections::HashSet;
    let mut seen = HashSet::new();
    results.retain(|pkg| seen.insert(pkg.name.clone()));
    Ok(results)
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
    if source == "official" || source == "unknown" {
        println!("{}", format!("Running 'sudo pacman -S {}'... (may prompt for password)", package).bold());
        let status = SysCommand::new("sudo")
            .args(["pacman", "-S", package])
            .status()
            .map_err(|e| format!("Failed to execute sudo pacman: {}", e))?;

        if status.success() {
            println!("{}", format!("Successfully installed '{}'", package).green());
            Ok(())
        } else {
            Err(format!(
                "Failed to install '{}'. Exit code: {}. Ensure sudo privileges and package availability.",
                package,
                status.code().unwrap_or(-1)
            ))
        }
    } else if source == "aur" {
        let helpers = [("yay", "yay -S"), ("paru", "paru -S")];
        for (helper, cmd) in helpers {
            if SysCommand::new("which").arg(helper).output().is_ok() {
                println!("{}", format!("Running '{} {}'... (may prompt for password)", cmd, package).bold());
                let status = SysCommand::new(helper)
                    .args(["-S", package])
                    .status()
                    .map_err(|e| format!("Failed to execute {}: {}", helper, e))?;

                if status.success() {
                    println!("{}", format!("Successfully installed '{}'", package).green());
                    return Ok(());
                } else {
                    return Err(format!(
                        "Failed to install '{}' with {}. Exit code: {}. Check AUR helper logs.",
                        package,
                        helper,
                        status.code().unwrap_or(-1)
                    ));
                }
            }
        }
        Err(format!(
            "Cannot install AUR package '{}'. Install an AUR helper like 'yay' or 'paru' (e.g., 'sudo pacman -S yay') and try again, or build manually from https://aur.archlinux.org/packages/{}",
            package, package
        ))
    } else {
        Err(format!("Unknown package source '{}'. Cannot install '{}'.", source, package))
    }
}
