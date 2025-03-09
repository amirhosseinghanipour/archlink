# archlink
`archLink` is a command-line tool for Arch Linux that assists users in finding and installing packages from the official repositories and the Arch User Repository (AUR). It provides suggestions for misspelled or vague packages queries using fuzzy matching and keyword relevance.

## Features
- Search for packages in official Arch repositories and AUR.
- Suggest packages based on fuzzy matching for misspelled names (e.g., "pythn" → "python").
- Rank suggestions using keyword relevance for vague queries (e.g., "network tool" → "nmap").
- Configurable maximum number of search results via a TOML file.

## Installation
### From Source
1. Clone the repository:
```git clone https://github.com/amirhosseinghanipour/archlink.git 
cd archLink
```
2. Build the project:
```
cargo build --release
```
3. Install manually:
```
sudo cp target/release/archlink /usr/local/bin
```

### Via AUR (Submitted)
1. Use an AUR helped to install
```
sudo pacman -S archlink
```
or 
```
yay -S archlink
```

## Prerequisites
- Arch Linux system (otherwise you're wasting your time here).
- `pacman` package manager installed.
- Rust and Cargo for building from source.
- Internet connection for AUR searches (real real).
- An AUR helper like yay or paru for installing AUR packages (**recommended**).

## Usage
### Search for Packages 
Search for packages and get suggestions:
```
arch link search <query>
```
Example:
```
arch linksearch python
```
Output:
```
Suggestions for 'python':
1. python                    - Next generation of the python high-level scripting language [official]
2. python-pip                - The PyPA recommended tool for installing Python packages [official]
...
Enter the number of the package to install (0 to exit):
```

### Install a Package Directly
Install a specific package without searching:
```
archlink install <package>
```
Example:
```
archlink install python
```
Output:
```
Running 'sudo pacman -S python'... (may prompt for password)
Successfully installed 'python'
```

### Help 
Display available commands and options:
```
archlink --help
```
Output:
```
archlink 0.1.0
ArchLink helps Arch Linux users to find the right packages to install

USAGE:
    archlink <SUBCOMMAND>

OPTIONS:
    -h, --help       Print help information
    -V, --version    Print version information

SUBCOMMANDS:
    search    Search for packages in official repos and AUR
    install   Install a package directly
```

## Configuration
`archlink` uses a configuration file located at `/etc/archlink/config.toml`. The default configuration is:
```
[default]
max_results = 10
```
### Customize
Edit `/etc/archlink/config.toml` to change the maximum number of search results:
```
[default]
max_results = 5
```
If the file is missing or malformed, `archlink` defaults to 10 results and prints a warning.


## Contributing
Contributions are welcome. To contribute:
1. Fork the repository on GitHub.
2. Clone your fork.
3. Create a branch for your changes.
4. Commit your changes.
5. Push to your fork.
6. Open a pull request on the main repository.
### Repoting issues 
File bug reports or feature requests in the Issues section.

## License
`archlink` is licensed under the MIT license.

## Building and Packaging
To create an Arch package:
1. Navigate to the project directory:
```
cd archlink
```
2. Build the package:
```
makekpg -si
```
The `PKGBUILD` file is included in the repository for AUR submission.

## Dependencies
- `clap` (CLI parsing)
- `reqwest` (HTTP requests for AUR)
- `serde` (JSON and TOML serialization)
- `tokio` (async runtime)
- `strsim` (fuzzy matching)
- `toml` (configuration parsing)
- `urlencoding` (URL encoding for AUR queries)
- `colored` (ANSI colors)
