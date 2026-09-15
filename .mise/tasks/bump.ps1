$ErrorActionPreference = "Stop"

$level = if ($args.Count -gt 0) { $args[0] } else { "" }

if ($level -notmatch '^(major|minor|patch)$') {
    Write-Error "Usage: .\bump.ps1 <major|minor|patch>"
    exit 1
}

cargo set-version --bump $level

$version = cargo metadata --no-deps --format-version 1 |
    ConvertFrom-Json |
    Select-Object -ExpandProperty packages |
    Select-Object -First 1 |
    Select-Object -ExpandProperty version

git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to $version"
